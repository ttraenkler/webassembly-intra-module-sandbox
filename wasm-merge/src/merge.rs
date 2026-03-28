use std::collections::{HashMap, HashSet};
use wasm_encoder::{
    CodeSection, DataSection, ElementSection, ExportKind, ExportSection, FunctionSection,
    GlobalSection, GlobalType, ImportSection, MemorySection, MemoryType, Module, TableSection,
    TableType, TypeSection,
    reencode::{self, Reencode},
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
/// If `exports_from` is Some(label), only re-export from the module with that label.
/// Otherwise, re-export from all modules.
pub fn merge(component: &Component, exports_from: Option<&str>) -> Result<Vec<u8>, String> {
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

    // ── Collect unresolved imports ─────────────────────────────────────
    // First pass: find all imports that can't be resolved, deduplicate.
    #[derive(Clone, Hash, Eq, PartialEq)]
    #[allow(dead_code)]
    struct UnresolvedImport {
        module: String,
        field: String,
        tyref: String, // serialized for dedup
    }
    let mut unresolved_funcs: Vec<(String, String, u32)> = Vec::new(); // (mod, field, type_idx from first module that imports it)
    let mut unresolved_mems: Vec<(String, String, wasmparser::MemoryType)> = Vec::new();
    let mut unresolved_globals: Vec<(String, String, wasmparser::GlobalType)> = Vec::new();
    let mut unresolved_tables: Vec<(String, String, wasmparser::TableType)> = Vec::new();
    let mut seen_unresolved: HashSet<(String, String)> = HashSet::new();

    for &mi in &module_order {
        let info = &infos[mi];
        for (mod_name, field, tyref) in &info.imports {
            let key = (mod_name.clone(), field.clone());
            if export_map.contains_key(&key) { continue; }
            if !seen_unresolved.insert(key.clone()) { continue; }
            match tyref {
                wasmparser::TypeRef::Func(ti) => { unresolved_funcs.push((mod_name.clone(), field.clone(), *ti)); }
                wasmparser::TypeRef::Memory(mt) => { unresolved_mems.push((mod_name.clone(), field.clone(), *mt)); }
                wasmparser::TypeRef::Global(gt) => { unresolved_globals.push((mod_name.clone(), field.clone(), *gt)); }
                wasmparser::TypeRef::Table(tt) => { unresolved_tables.push((mod_name.clone(), field.clone(), *tt)); }
                _ => {}
            }
        }
    }

    let unresolved_func_count = unresolved_funcs.len() as u32;
    let unresolved_mem_count = unresolved_mems.len() as u32;
    let unresolved_global_count = unresolved_globals.len() as u32;
    let unresolved_table_count = unresolved_tables.len() as u32;

    // Build lookup: (mod, field) → new import index
    let mut unresolved_func_map: HashMap<(String, String), u32> = HashMap::new();
    for (i, (m, f, _)) in unresolved_funcs.iter().enumerate() {
        unresolved_func_map.insert((m.clone(), f.clone()), i as u32);
    }
    let mut unresolved_mem_map: HashMap<(String, String), u32> = HashMap::new();
    for (i, (m, f, _)) in unresolved_mems.iter().enumerate() {
        unresolved_mem_map.insert((m.clone(), f.clone()), i as u32);
    }
    let mut unresolved_global_map: HashMap<(String, String), u32> = HashMap::new();
    for (i, (m, f, _)) in unresolved_globals.iter().enumerate() {
        unresolved_global_map.insert((m.clone(), f.clone()), i as u32);
    }
    let mut unresolved_table_map: HashMap<(String, String), u32> = HashMap::new();
    for (i, (m, f, _)) in unresolved_tables.iter().enumerate() {
        unresolved_table_map.insert((m.clone(), f.clone()), i as u32);
    }

    // Offsets: imports first, then defined items
    let mut offsets: Vec<Offsets> = Vec::new();
    let (mut t, mut f, mut m, mut tb, mut g) = (
        0u32,
        unresolved_func_count,
        unresolved_mem_count,
        unresolved_table_count,
        unresolved_global_count,
    );

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
                    } else if let Some(&new_idx) = unresolved_func_map.get(&key) {
                        r.func_map.insert(imp_f, new_idx);
                    }
                    imp_f += 1;
                }
                wasmparser::TypeRef::Memory(_) => {
                    if let Some((tp, _, ti)) = resolved {
                        let target = &infos[module_order[*tp]];
                        let target_off = &offsets[*tp];
                        r.mem_map
                            .insert(imp_m, target_off.mem_base + (ti - target.imported_mem_count));
                    } else if let Some(&new_idx) = unresolved_mem_map.get(&key) {
                        r.mem_map.insert(imp_m, new_idx);
                    }
                    imp_m += 1;
                }
                wasmparser::TypeRef::Global(_) => {
                    if let Some((tp, _, ti)) = resolved {
                        let target = &infos[module_order[*tp]];
                        let target_off = &offsets[*tp];
                        r.global_map
                            .insert(imp_g, target_off.global_base + (ti - target.imported_global_count));
                    } else if let Some(&new_idx) = unresolved_global_map.get(&key) {
                        r.global_map.insert(imp_g, new_idx);
                    }
                    imp_g += 1;
                }
                wasmparser::TypeRef::Table(_) => {
                    if let Some((tp, _, ti)) = resolved {
                        let target = &infos[module_order[*tp]];
                        let target_off = &offsets[*tp];
                        r.table_map
                            .insert(imp_t, target_off.table_base + (ti - target.imported_table_count));
                    } else if let Some(&new_idx) = unresolved_table_map.get(&key) {
                        r.table_map.insert(imp_t, new_idx);
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
    let mut seen_exports: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (pos, &mi) in module_order.iter().enumerate() {
        let remap = &mut remappers[pos];
        let label = module_label.get(&mi).cloned().unwrap_or(format!("m{mi}"));

        // If --exports-from is set, only export from matching module.
        // Match against: module_label, CLI position ("0", "1", ...), or module index.
        if let Some(from) = exports_from {
            let pos_str = format!("{pos}");
            let mi_str = format!("{mi}");
            if label != from && pos_str != from && mi_str != from { continue; }
        }
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
            // Deduplicate: prefix with label on collision
            let export_name = if seen_exports.contains(name) {
                format!("{label}_{name}")
            } else {
                name.clone()
            };
            seen_exports.insert(export_name.clone());
            export_sec.export(&export_name, ek, new_idx);
        }
    }

    // Assemble
    let mut output = Module::new();
    output.section(&type_sec);

    // Emit unresolved imports
    if unresolved_func_count + unresolved_mem_count + unresolved_global_count + unresolved_table_count > 0 {
        let mut imp_sec = ImportSection::new();
        for (mod_name, field, orig_type_idx) in &unresolved_funcs {
            // Find the first module that has this import and remap its type index
            let mut new_ti = *orig_type_idx;
            for (pos, &mi) in module_order.iter().enumerate() {
                for (mn, fn_, tr) in &infos[mi].imports {
                    if mn == mod_name && fn_ == field {
                        if let wasmparser::TypeRef::Func(ti) = tr {
                            new_ti = remappers[pos].type_index(*ti);
                        }
                        break;
                    }
                }
            }
            imp_sec.import(mod_name, field, wasm_encoder::EntityType::Function(new_ti));
        }
        for (mod_name, field, mt) in &unresolved_mems {
            imp_sec.import(mod_name, field, wasm_encoder::EntityType::Memory(MemoryType {
                minimum: mt.initial, maximum: mt.maximum, memory64: mt.memory64,
                shared: mt.shared, page_size_log2: mt.page_size_log2,
            }));
        }
        for (mod_name, field, gt) in &unresolved_globals {
            imp_sec.import(mod_name, field, wasm_encoder::EntityType::Global(GlobalType {
                val_type: reencode::RoundtripReencoder.val_type(gt.content_type).unwrap(),
                mutable: gt.mutable, shared: gt.shared,
            }));
        }
        for (mod_name, field, tt) in &unresolved_tables {
            imp_sec.import(mod_name, field, wasm_encoder::EntityType::Table(TableType {
                element_type: wasm_encoder::RefType::FUNCREF,
                table64: tt.table64, minimum: tt.initial as u64,
                maximum: tt.maximum.map(|m| m as u64), shared: false,
            }));
        }
        output.section(&imp_sec);
    }

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
