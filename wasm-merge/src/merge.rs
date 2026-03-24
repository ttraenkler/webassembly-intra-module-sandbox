use std::collections::HashMap;
use wasm_encoder::{
    CodeSection, DataSection, ElementSection, ExportKind, ExportSection, FunctionSection,
    GlobalSection, MemorySection, Module, TableSection, TypeSection,
    reencode::Reencode,
};
use wasmparser::{Parser, Payload};

use crate::extract::Component;

/// Index remapper that implements the Reencode trait.
/// Override only the index-mapping methods; the trait handles all operator encoding.
struct Remapper {
    func_map: HashMap<u32, u32>,
    mem_map: HashMap<u32, u32>,
    type_map: HashMap<u32, u32>,
    global_map: HashMap<u32, u32>,
    table_map: HashMap<u32, u32>,
}

impl Reencode for Remapper {
    type Error = wasm_encoder::reencode::Error;

    fn function_index(&mut self, idx: u32) -> u32 {
        self.func_map.get(&idx).copied().unwrap_or(idx)
    }
    fn memory_index(&mut self, idx: u32) -> u32 {
        self.mem_map.get(&idx).copied().unwrap_or(idx)
    }
    fn type_index(&mut self, idx: u32) -> u32 {
        self.type_map.get(&idx).copied().unwrap_or(idx)
    }
    fn global_index(&mut self, idx: u32) -> u32 {
        self.global_map.get(&idx).copied().unwrap_or(idx)
    }
    fn table_index(&mut self, idx: u32) -> u32 {
        self.table_map.get(&idx).copied().unwrap_or(idx)
    }
}

/// Lightweight info extracted from a parsed core module.
struct ModuleInfo {
    type_count: u32,
    func_count: u32, // defined (non-imported) functions
    mem_count: u32,
    table_count: u32,
    global_count: u32,
    imported_func_count: u32,
    imported_mem_count: u32,
    imported_table_count: u32,
    imported_global_count: u32,
    exports: Vec<(String, wasmparser::ExternalKind, u32)>,
    imports: Vec<(String, String, wasmparser::TypeRef)>,
}

fn scan_module(wasm: &[u8]) -> Result<ModuleInfo, String> {
    let parser = Parser::new(0);
    let mut info = ModuleInfo {
        type_count: 0,
        func_count: 0,
        mem_count: 0,
        table_count: 0,
        global_count: 0,
        imported_func_count: 0,
        imported_mem_count: 0,
        imported_table_count: 0,
        imported_global_count: 0,
        exports: Vec::new(),
        imports: Vec::new(),
    };

    for payload in parser.parse_all(wasm) {
        let payload = payload.map_err(|e| format!("Parse: {e}"))?;
        match payload {
            Payload::TypeSection(reader) => {
                for rg in reader {
                    let rg = rg.map_err(|e| format!("Type: {e}"))?;
                    info.type_count += rg.types().len() as u32;
                }
            }
            Payload::ImportSection(reader) => {
                for imp in reader {
                    let imp = imp.map_err(|e| format!("Import: {e}"))?;
                    match &imp.ty {
                        wasmparser::TypeRef::Func(_) => info.imported_func_count += 1,
                        wasmparser::TypeRef::Memory(_) => info.imported_mem_count += 1,
                        wasmparser::TypeRef::Table(_) => info.imported_table_count += 1,
                        wasmparser::TypeRef::Global(_) => info.imported_global_count += 1,
                        _ => {}
                    }
                    info.imports.push((
                        imp.module.to_string(),
                        imp.name.to_string(),
                        imp.ty,
                    ));
                }
            }
            Payload::FunctionSection(reader) => {
                info.func_count = reader.count();
            }
            Payload::MemorySection(reader) => {
                info.mem_count = reader.count();
            }
            Payload::TableSection(reader) => {
                info.table_count = reader.count();
            }
            Payload::GlobalSection(reader) => {
                info.global_count = reader.count();
            }
            Payload::ExportSection(reader) => {
                for e in reader {
                    let e = e.map_err(|e| format!("Export: {e}"))?;
                    info.exports.push((e.name.to_string(), e.kind, e.index));
                }
            }
            _ => {}
        }
    }

    Ok(info)
}

/// Merge the component's core modules into a single multi-memory core module.
pub fn merge(component: &Component) -> Result<Vec<u8>, String> {
    let infos: Vec<ModuleInfo> = component
        .modules
        .iter()
        .map(|m| scan_module(&m.wasm))
        .collect::<Result<Vec<_>, _>>()?;

    // Determine merge order by reversing instantiation order: the last-instantiated
    // module (the leaf/consumer) goes first so its memory gets index 0 in the
    // merged output, and dependencies come after with higher memory indices.
    let mut module_order: Vec<usize> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut instance_to_module: HashMap<u32, usize> = HashMap::new();

    for (inst_idx, inst) in component.instances.iter().enumerate() {
        instance_to_module.insert(inst_idx as u32, inst.module_idx as usize);
    }

    for inst in component.instances.iter().rev() {
        let mi = inst.module_idx as usize;
        if seen.insert(mi) {
            module_order.push(mi);
        }
    }
    for i in 0..infos.len() {
        if seen.insert(i) {
            module_order.push(i);
        }
    }

    // Build namespace labels for import resolution
    let mut module_label: HashMap<usize, String> = HashMap::new();
    for inst in &component.instances {
        for (namespace, src_instance_idx) in &inst.args {
            if let Some(&mod_idx) = instance_to_module.get(src_instance_idx) {
                module_label.insert(mod_idx, namespace.clone());
            }
        }
    }

    // Export map: (label, export_name) -> (merge_position, kind, index)
    let mut export_map: HashMap<(String, String), (usize, wasmparser::ExternalKind, u32)> =
        HashMap::new();
    for (pos, &mi) in module_order.iter().enumerate() {
        let label = module_label.get(&mi).cloned().unwrap_or(format!("m{mi}"));
        for (name, kind, idx) in &infos[mi].exports {
            export_map.insert((label.clone(), name.clone()), (pos, *kind, *idx));
        }
    }

    // Calculate per-module offsets in the merged index spaces
    struct Offsets {
        type_base: u32,
        func_base: u32,
        mem_base: u32,
        table_base: u32,
        global_base: u32,
    }

    let mut offsets: Vec<Offsets> = Vec::new();
    let (mut t, mut f, mut m, mut tb, mut g) = (0u32, 0u32, 0u32, 0u32, 0u32);

    for &mi in &module_order {
        offsets.push(Offsets {
            type_base: t,
            func_base: f,
            mem_base: m,
            table_base: tb,
            global_base: g,
        });
        t += infos[mi].type_count;
        f += infos[mi].func_count;
        m += infos[mi].mem_count;
        tb += infos[mi].table_count;
        g += infos[mi].global_count;
    }

    // Build a Remapper for each module
    let mut remappers: Vec<Remapper> = Vec::new();

    for (pos, &mi) in module_order.iter().enumerate() {
        let info = &infos[mi];
        let off = &offsets[pos];
        let mut r = Remapper {
            func_map: HashMap::new(),
            mem_map: HashMap::new(),
            type_map: HashMap::new(),
            global_map: HashMap::new(),
            table_map: HashMap::new(),
        };

        // Types
        for i in 0..info.type_count {
            r.type_map.insert(i, off.type_base + i);
        }

        // Resolve imports → target module's exports
        let mut imp_f = 0u32;
        let mut imp_m = 0u32;
        let mut imp_g = 0u32;
        let mut imp_t = 0u32;

        for (mod_name, field, tyref) in &info.imports {
            let key = (mod_name.clone(), field.clone());
            let resolved = export_map.get(&key);
            if resolved.is_none() {
                eprintln!(
                    "  warning: unresolved import \"{mod_name}\".\"{field}\" in module {mi}"
                );
            }
            match tyref {
                wasmparser::TypeRef::Func(_) => {
                    if let Some((tp, _, ti)) = resolved {
                        let target = &infos[module_order[*tp]];
                        let target_off = &offsets[*tp];
                        r.func_map
                            .insert(imp_f, target_off.func_base + (ti - target.imported_func_count));
                    }
                    imp_f += 1;
                }
                wasmparser::TypeRef::Memory(_) => {
                    if let Some((tp, _, ti)) = resolved {
                        let target = &infos[module_order[*tp]];
                        let target_off = &offsets[*tp];
                        r.mem_map
                            .insert(imp_m, target_off.mem_base + (ti - target.imported_mem_count));
                    }
                    imp_m += 1;
                }
                wasmparser::TypeRef::Global(_) => {
                    if let Some((tp, _, ti)) = resolved {
                        let target = &infos[module_order[*tp]];
                        let target_off = &offsets[*tp];
                        r.global_map
                            .insert(imp_g, target_off.global_base + (ti - target.imported_global_count));
                    }
                    imp_g += 1;
                }
                wasmparser::TypeRef::Table(_) => {
                    if let Some((tp, _, ti)) = resolved {
                        let target = &infos[module_order[*tp]];
                        let target_off = &offsets[*tp];
                        r.table_map
                            .insert(imp_t, target_off.table_base + (ti - target.imported_table_count));
                    }
                    imp_t += 1;
                }
                _ => {}
            }
        }

        // Defined (non-imported) items
        for i in 0..info.func_count {
            r.func_map.insert(info.imported_func_count + i, off.func_base + i);
        }
        for i in 0..info.mem_count {
            r.mem_map.insert(info.imported_mem_count + i, off.mem_base + i);
        }
        for i in 0..info.global_count {
            r.global_map.insert(info.imported_global_count + i, off.global_base + i);
        }
        for i in 0..info.table_count {
            r.table_map.insert(info.imported_table_count + i, off.table_base + i);
        }

        remappers.push(r);
    }

    // ── Emit merged module using Reencode ───────────────────────────────

    let mut type_sec = TypeSection::new();
    let mut func_sec = FunctionSection::new();
    let mut mem_sec = MemorySection::new();
    let mut table_sec = TableSection::new();
    let mut global_sec = GlobalSection::new();
    let mut export_sec = ExportSection::new();
    let mut code_sec = CodeSection::new();
    let mut data_sec = DataSection::new();
    let mut elem_sec = ElementSection::new();

    for (pos, &mi) in module_order.iter().enumerate() {
        let wasm = &component.modules[mi].wasm;
        let remap = &mut remappers[pos];
        let parser = Parser::new(0);

        for payload in parser.parse_all(wasm) {
            let payload = payload.map_err(|e| format!("Parse: {e}"))?;
            match payload {
                Payload::TypeSection(reader) => {
                    remap
                        .parse_type_section(&mut type_sec, reader)
                        .map_err(|e| format!("Type: {e}"))?;
                }
                Payload::FunctionSection(reader) => {
                    remap
                        .parse_function_section(&mut func_sec, reader)
                        .map_err(|e| format!("Func: {e}"))?;
                }
                Payload::MemorySection(reader) => {
                    remap
                        .parse_memory_section(&mut mem_sec, reader)
                        .map_err(|e| format!("Mem: {e}"))?;
                }
                Payload::TableSection(reader) => {
                    remap
                        .parse_table_section(&mut table_sec, reader)
                        .map_err(|e| format!("Table: {e}"))?;
                }
                Payload::GlobalSection(reader) => {
                    remap
                        .parse_global_section(&mut global_sec, reader)
                        .map_err(|e| format!("Global: {e}"))?;
                }
                Payload::CodeSectionEntry(body) => {
                    remap
                        .parse_function_body(&mut code_sec, body)
                        .map_err(|e| format!("Code: {e}"))?;
                }
                Payload::DataSection(reader) => {
                    remap
                        .parse_data_section(&mut data_sec, reader)
                        .map_err(|e| format!("Data: {e}"))?;
                }
                Payload::ElementSection(reader) => {
                    remap
                        .parse_element_section(&mut elem_sec, reader)
                        .map_err(|e| format!("Elem: {e}"))?;
                }
                // Skip import sections — all imports are resolved internally
                Payload::ImportSection(_) => {}
                _ => {}
            }
        }
    }

    // Exports: remap indices through each module's remapper
    for (pos, &mi) in module_order.iter().enumerate() {
        let remap = &mut remappers[pos];
        for (name, kind, idx) in &infos[mi].exports {
            let new_idx = match kind {
                wasmparser::ExternalKind::Func => remap.function_index(*idx),
                wasmparser::ExternalKind::Memory => remap.memory_index(*idx),
                wasmparser::ExternalKind::Table => remap.table_index(*idx),
                wasmparser::ExternalKind::Global => remap.global_index(*idx),
                _ => *idx,
            };
            let ek = match kind {
                wasmparser::ExternalKind::Func => ExportKind::Func,
                wasmparser::ExternalKind::Memory => ExportKind::Memory,
                wasmparser::ExternalKind::Table => ExportKind::Table,
                wasmparser::ExternalKind::Global => ExportKind::Global,
                _ => continue,
            };
            export_sec.export(name, ek, new_idx);
        }
    }

    // Assemble
    let mut output = Module::new();
    output.section(&type_sec);
    output.section(&func_sec);
    if tb > 0 {
        output.section(&table_sec);
    }
    output.section(&mem_sec);
    if g > 0 {
        output.section(&global_sec);
    }
    output.section(&export_sec);
    if !elem_sec.is_empty() {
        output.section(&elem_sec);
    }
    output.section(&code_sec);
    output.section(&data_sec);

    Ok(output.finish())
}
