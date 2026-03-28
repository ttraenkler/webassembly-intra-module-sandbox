//! Dispatch merge: analyze the library, thread $instance_idx through
//! state-touching functions, generate br_table dispatch wrappers.
//! One copy of each function body — smallest binary, dispatch overhead per memory access.

use std::collections::{HashMap, HashSet};
use wasm_encoder::{
    CodeSection, ConstExpr, DataSection, DataSegment, DataSegmentMode, ExportKind, ExportSection,
    Function, FunctionSection, GlobalSection, GlobalType, ImportSection, MemoryType,
    Module, TableSection, TableType, TypeSection, ValType,
    reencode::{self, Reencode},
};
use wasmparser::{Operator, Parser, Payload};

use crate::extract::Component;
use crate::specialize::analyze_lib;

/// Dispatch merge: merge consumers + library with br_table dispatch wrappers.
pub fn dispatch_merge(
    component: &Component,
    lib_idx: usize,
    consumer_indices: &[usize],
    _exports_from: Option<usize>,
) -> Result<Vec<u8>, String> {
    let lib_wasm = &component.modules[lib_idx].wasm;
    let analysis = analyze_lib(lib_wasm)?;
    let mutable_set: HashSet<u32> = analysis.mutable_globals.iter().copied().collect();
    let vt = |v: wasmparser::ValType| reencode::RoundtripReencoder.val_type(v).unwrap();

    let n = consumer_indices.len() as u32;

    eprintln!("  Dispatch: {} state-touching, {} pure, {} consumers",
        analysis.state_touching.len(),
        analysis.num_functions as usize - analysis.state_touching.len(),
        n);

    // ── Parse library sections ─────────────────────────────────────────
    let mut orig_types: Vec<wasmparser::FuncType> = Vec::new();
    let mut orig_imports: Vec<wasmparser::Import> = Vec::new();
    let mut orig_func_type_indices: Vec<u32> = Vec::new();
    let mut orig_globals: Vec<(wasmparser::GlobalType, wasmparser::ConstExpr)> = Vec::new();
    let mut orig_memories: Vec<wasmparser::MemoryType> = Vec::new();
    let mut orig_tables: Vec<wasmparser::TableType> = Vec::new();
    let mut orig_exports: Vec<wasmparser::Export> = Vec::new();
    let mut orig_data: Vec<wasmparser::Data> = Vec::new();
    let mut orig_bodies: Vec<wasmparser::FunctionBody> = Vec::new();
    let mut orig_elements: Vec<wasmparser::Element> = Vec::new();

    let parser = Parser::new(0);
    for payload in parser.parse_all(lib_wasm) {
        let payload = payload.map_err(|e| format!("{e}"))?;
        match payload {
            Payload::TypeSection(r) => { for t in r.into_iter_err_on_gc_types() { orig_types.push(t.map_err(|e| format!("{e}"))?); } }
            Payload::ImportSection(r) => { for i in r { orig_imports.push(i.map_err(|e| format!("{e}"))?); } }
            Payload::FunctionSection(r) => { for t in r { orig_func_type_indices.push(t.map_err(|e| format!("{e}"))?); } }
            Payload::GlobalSection(r) => { for g in r { let g = g.map_err(|e| format!("{e}"))?; orig_globals.push((g.ty, g.init_expr)); } }
            Payload::MemorySection(r) => { for m in r { orig_memories.push(m.map_err(|e| format!("{e}"))?); } }
            Payload::TableSection(r) => { for t in r { let t = t.map_err(|e| format!("{e}"))?; orig_tables.push(t.ty); } }
            Payload::ExportSection(r) => { for e in r { orig_exports.push(e.map_err(|e| format!("{e}"))?); } }
            Payload::DataSection(r) => { for d in r { orig_data.push(d.map_err(|e| format!("{e}"))?); } }
            Payload::ElementSection(r) => { for e in r { orig_elements.push(e.map_err(|e| format!("{e}"))?); } }
            Payload::CodeSectionEntry(body) => { orig_bodies.push(body); }
            _ => {}
        }
    }

    // ── Collect unresolved consumer imports (e.g. WASI) ────────────────
    // These need to be emitted as imports in the merged module.
    let lib_export_names: HashSet<&str> = analysis.exports.iter().map(|(n, _)| n.as_str()).collect();
    let mut unresolved_imports: Vec<(String, String, u32)> = Vec::new(); // (module, name, type_idx in consumer)
    let mut unresolved_map: HashMap<(String, String), u32> = HashMap::new(); // key → merged func idx (filled later)
    let mut unresolved_consumer_types: Vec<wasmparser::FuncType> = Vec::new();

    for &ci in consumer_indices {
        let parser = Parser::new(0);
        let mut consumer_types: Vec<wasmparser::FuncType> = Vec::new();
        for payload in parser.parse_all(&component.modules[ci].wasm) {
            let payload = payload.map_err(|e| format!("{e}"))?;
            match payload {
                Payload::TypeSection(r) => {
                    for t in r.into_iter_err_on_gc_types() { consumer_types.push(t.map_err(|e| format!("{e}"))?); }
                }
                Payload::ImportSection(r) => {
                    for imp in r {
                        let imp = imp.map_err(|e| format!("{e}"))?;
                        if let wasmparser::TypeRef::Func(ti) = imp.ty {
                            let key = (imp.module.to_string(), imp.name.to_string());
                            if !lib_export_names.contains(imp.name) && !unresolved_map.contains_key(&key) {
                                let idx = unresolved_imports.len() as u32;
                                unresolved_map.insert(key, idx); // placeholder — shifted later
                                unresolved_consumer_types.push(consumer_types[ti as usize].clone());
                                unresolved_imports.push((imp.module.to_string(), imp.name.to_string(), ti));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let unresolved_func_count = unresolved_imports.len() as u32;
    // Unresolved imports get func indices 0..unresolved_func_count
    // All lib function indices shift by unresolved_func_count
    let fshift = unresolved_func_count;

    // ── Global layout: mutable × N, immutable × 1, then N immutable $inst globals
    let mut new_global_base: HashMap<u32, u32> = HashMap::new();
    let mut next_global = 0u32;
    for (idx, (gt, _)) in orig_globals.iter().enumerate() {
        new_global_base.insert(idx as u32, next_global);
        next_global += if gt.mutable { n } else { 1 };
    }
    let inst_global_base = next_global; // immutable $inst0=0, $inst1=1, ...

    // ── Scan needed dispatch wrappers ──────────────────────────────────
    let mut need_mem_ops: HashSet<String> = HashSet::new();
    let mut need_gg: HashSet<u32> = HashSet::new();
    let mut need_gs: HashSet<u32> = HashSet::new();
    for (i, body) in orig_bodies.iter().enumerate() {
        let fi = analysis.num_imported_functions + i as u32;
        if !analysis.state_touching.contains(&fi) { continue; }
        if let Ok(ops) = body.get_operators_reader() {
            for op in ops {
                if let Ok(op) = op {
                    if let Some(name) = mem_op_name(&op) { need_mem_ops.insert(name); }
                    match op {
                        Operator::GlobalGet { global_index } if mutable_set.contains(&global_index) => { need_gg.insert(global_index); }
                        Operator::GlobalSet { global_index } if mutable_set.contains(&global_index) => { need_gs.insert(global_index); }
                        _ => {}
                    }
                }
            }
        }
    }

    // ── Types ──────────────────────────────────────────────────────────
    let mut type_sec = TypeSection::new();
    for ft in &orig_types {
        type_sec.ty().function(ft.params().iter().map(|t| vt(*t)), ft.results().iter().map(|t| vt(*t)));
    }
    let mut nti = orig_types.len() as u32;

    // Augmented types: original + $idx appended
    let mut aug_type: HashMap<u32, u32> = HashMap::new();
    for fi in 0..analysis.num_functions {
        if !analysis.state_touching.contains(&fi) { continue; }
        let oti = analysis.func_type_indices[fi as usize];
        if aug_type.contains_key(&oti) { continue; }
        let ft = &orig_types[oti as usize];
        let mut p: Vec<ValType> = ft.params().iter().map(|t| vt(*t)).collect();
        p.push(ValType::I32);
        type_sec.ty().function(p, ft.results().iter().map(|t| vt(*t)));
        aug_type.insert(oti, nti);
        nti += 1;
    }

    // Dispatch wrapper types — $idx as last param
    let mut mem_wrapper_types: HashMap<String, u32> = HashMap::new();
    for name in &need_mem_ops {
        let (mut params, results) = mem_op_sig(name);
        params.push(ValType::I32);
        type_sec.ty().function(params, results);
        mem_wrapper_types.insert(name.clone(), nti);
        nti += 1;
    }
    let ty_gg = nti; type_sec.ty().function([ValType::I32], [ValType::I32]); nti += 1;
    let ty_gs = nti; type_sec.ty().function([ValType::I32, ValType::I32], []); let _ = nti + 1;

    // Unresolved consumer import types
    let mut unresolved_type_indices: Vec<u32> = Vec::new();
    for ft in &unresolved_consumer_types {
        let ti = type_sec.len();
        type_sec.ty().function(ft.params().iter().map(|t| vt(*t)), ft.results().iter().map(|t| vt(*t)));
        unresolved_type_indices.push(ti);
    }

    // Also copy consumer types
    let mut consumer_type_offsets: Vec<u32> = Vec::new();
    for &ci in consumer_indices {
        consumer_type_offsets.push(type_sec.len());
        let parser = Parser::new(0);
        for payload in parser.parse_all(&component.modules[ci].wasm) {
            if let Ok(Payload::TypeSection(reader)) = payload {
                for ty in reader.into_iter_err_on_gc_types() {
                    if let Ok(ft) = ty {
                        type_sec.ty().function(ft.params().iter().map(|t| vt(*t)), ft.results().iter().map(|t| vt(*t)));
                    }
                }
            }
        }
    }

    // ── Functions ──────────────────────────────────────────────────────
    let mut func_sec = FunctionSection::new();
    // Library defined functions — state-touching get augmented type
    for (i, &oti) in orig_func_type_indices.iter().enumerate() {
        let fi = analysis.num_imported_functions + i as u32;
        func_sec.function(if analysis.state_touching.contains(&fi) { aug_type[&oti] } else { oti });
    }

    let mut nw = fshift + analysis.num_imported_functions + orig_func_type_indices.len() as u32;

    // Dispatch wrappers
    let mut mem_wrappers: HashMap<String, u32> = HashMap::new();
    for name in &need_mem_ops { func_sec.function(mem_wrapper_types[name]); mem_wrappers.insert(name.clone(), nw); nw += 1; }
    let mut w_gg: HashMap<u32, u32> = HashMap::new();
    let mut w_gs: HashMap<u32, u32> = HashMap::new();
    for &g in &need_gg { func_sec.function(ty_gg); w_gg.insert(g, nw); nw += 1; }
    for &g in &need_gs { func_sec.function(ty_gs); w_gs.insert(g, nw); nw += 1; }

    // Shell wrappers: N per exported state-touching function
    let mut st_exports: Vec<(String, u32)> = Vec::new();
    for e in &orig_exports {
        if let wasmparser::ExternalKind::Func = e.kind {
            if analysis.state_touching.contains(&e.index) {
                st_exports.push((e.name.to_string(), e.index));
            }
        }
    }
    let mut shell_wrappers: Vec<Vec<u32>> = Vec::new();
    for (_, fi) in &st_exports {
        let oti = orig_func_type_indices[(*fi - analysis.num_imported_functions) as usize];
        let mut iw = Vec::new();
        for _ in 0..n { func_sec.function(oti); iw.push(nw); nw += 1; }
        shell_wrappers.push(iw);
    }

    // Consumer functions
    let mut consumer_func_offsets: Vec<u32> = Vec::new();
    for &ci in consumer_indices {
        consumer_func_offsets.push(nw);
        let parser = Parser::new(0);
        for payload in parser.parse_all(&component.modules[ci].wasm) {
            if let Ok(Payload::FunctionSection(reader)) = payload {
                for t in reader {
                    if let Ok(ti) = t {
                        func_sec.function(consumer_type_offsets[consumer_indices.iter().position(|&x| x == ci).unwrap()] + ti);
                        nw += 1;
                    }
                }
            }
        }
    }

    // ── Code section ───────────────────────────────────────────────────
    use wasm_encoder::Instruction as I;
    let mut code = CodeSection::new();

    // Library function bodies — rewrite state-touching with dispatch
    for (i, body) in orig_bodies.iter().enumerate() {
        let fi = analysis.num_imported_functions + i as u32;
        let touching = analysis.state_touching.contains(&fi);

        let mut lgroups: Vec<(u32, wasmparser::ValType)> = Vec::new();
        for l in body.get_locals_reader().map_err(|e| format!("{e}"))? { lgroups.push(l.map_err(|e| format!("{e}"))?); }

        let oti = analysis.func_type_indices[fi as usize];
        let opc = orig_types[oti as usize].params().len() as u32;
        let idx_local = opc; // $idx appended as last param

        let mut enc_locals: Vec<(u32, ValType)> = lgroups.iter().map(|(c, v)| (*c, vt(*v))).collect();
        let declared_count: u32 = lgroups.iter().map(|(c, _)| c).sum();
        let temp_i32 = if touching { opc + 1 + declared_count } else { 0 };
        let temp_i64 = temp_i32 + 1;
        let temp_f32 = temp_i64 + 1;
        let temp_f64 = temp_f32 + 1;
        if touching { enc_locals.push((1, ValType::I32)); enc_locals.push((1, ValType::I64)); enc_locals.push((1, ValType::F32)); enc_locals.push((1, ValType::F64)); }

        let mut f = Function::new(enc_locals);
        let mut reader = body.get_operators_reader().map_err(|e| format!("{e}"))?;

        while !reader.eof() {
            let op = reader.read().map_err(|e| format!("{e}"))?;

            if !touching {
                match op {
                    Operator::Call { function_index } if fshift > 0 => {
                        f.instruction(&I::Call(function_index + fshift));
                    }
                    _ => {
                        f.instruction(&reencode::RoundtripReencoder.instruction(op).map_err(|e| format!("{e}"))?);
                    }
                }
                continue;
            }

            // Memory op → dispatch wrapper call with offset handling
            if let Some((name, offset)) = mem_op_name_offset(&op) {
                if offset != 0 {
                    let temp = match name.as_str() {
                        n if n.starts_with("i64.store") => Some(temp_i64),
                        n if n.starts_with("f32.store") => Some(temp_f32),
                        n if n.starts_with("f64.store") => Some(temp_f64),
                        n if n.starts_with("i32.store") => Some(temp_i32),
                        _ => None,
                    };
                    if let Some(t) = temp {
                        f.instruction(&I::LocalSet(t));
                        f.instruction(&I::I32Const(offset as i32));
                        f.instruction(&I::I32Add);
                        f.instruction(&I::LocalGet(t));
                    } else {
                        f.instruction(&I::I32Const(offset as i32));
                        f.instruction(&I::I32Add);
                    }
                }
                f.instruction(&I::LocalGet(idx_local));
                f.instruction(&I::Call(mem_wrappers[&name]));
                continue;
            }

            match op {
                Operator::GlobalGet { global_index } if mutable_set.contains(&global_index) => {
                    f.instruction(&I::LocalGet(idx_local));
                    f.instruction(&I::Call(w_gg[&global_index]));
                }
                Operator::GlobalSet { global_index } if mutable_set.contains(&global_index) => {
                    f.instruction(&I::LocalGet(idx_local));
                    f.instruction(&I::Call(w_gs[&global_index]));
                }
                Operator::Call { function_index } if analysis.state_touching.contains(&function_index) => {
                    f.instruction(&I::LocalGet(idx_local));
                    f.instruction(&I::Call(function_index + fshift));
                }
                Operator::Call { function_index } => {
                    f.instruction(&I::Call(function_index + fshift));
                }
                Operator::LocalGet { local_index } if local_index >= opc => {
                    f.instruction(&I::LocalGet(local_index + 1));
                }
                Operator::LocalSet { local_index } if local_index >= opc => {
                    f.instruction(&I::LocalSet(local_index + 1));
                }
                Operator::LocalTee { local_index } if local_index >= opc => {
                    f.instruction(&I::LocalTee(local_index + 1));
                }
                _ => {
                    f.instruction(&reencode::RoundtripReencoder.instruction(op).map_err(|e| format!("{e}"))?);
                }
            }
        }
        code.function(&f);
    }

    // Dispatch wrapper bodies — $idx is last param, br_table dispatch
    for name in &need_mem_ops {
        let (params, _) = mem_op_sig(name);
        let idx_param = params.len() as u32;
        code.function(&gen_mem_dispatch(n, idx_param, name, params.len() as u32));
    }
    for &g in &need_gg {
        let base = new_global_base[&g];
        code.function(&gen_global_get_dispatch(n, 0, base));
    }
    for &g in &need_gs {
        let base = new_global_base[&g];
        code.function(&gen_global_set_dispatch(n, 1, base));
    }

    // Shell wrappers — pass immutable global.get $instN
    for (_ei, (_, fi)) in st_exports.iter().enumerate() {
        let oti = orig_func_type_indices[(*fi - analysis.num_imported_functions) as usize];
        let ft = &orig_types[oti as usize];
        for inst in 0..n {
            let mut f = Function::new([]);
            for p in 0..ft.params().len() { f.instruction(&I::LocalGet(p as u32)); }
            f.instruction(&I::GlobalGet(inst_global_base + inst));
            f.instruction(&I::Call(*fi + fshift));
            f.instruction(&I::End);
            code.function(&f);
        }
    }

    // Consumer function bodies with import resolution
    for (ci_ord, &ci) in consumer_indices.iter().enumerate() {
        let consumer_wasm = &component.modules[ci].wasm;
        let mut consumer_import_remap: HashMap<u32, u32> = HashMap::new();
        let mut consumer_imp_func_count = 0u32;
        {
            let parser = Parser::new(0);
            for payload in parser.parse_all(consumer_wasm) {
                if let Ok(Payload::ImportSection(reader)) = payload {
                    for imp in reader {
                        if let Ok(imp) = imp {
                            if let wasmparser::TypeRef::Func(_) = imp.ty {
                                if let Some(ei) = st_exports.iter().position(|(nm, _)| nm == imp.name) {
                                    consumer_import_remap.insert(consumer_imp_func_count, shell_wrappers[ei][ci_ord]);
                                } else if let Some((_, eidx)) = analysis.exports.iter().find(|(nm, _)| nm == imp.name) {
                                    consumer_import_remap.insert(consumer_imp_func_count, *eidx + fshift);
                                } else {
                                    // Unresolved import (e.g. WASI) — map to merged import slot
                                    let key = (imp.module.to_string(), imp.name.to_string());
                                    if let Some(&merged_idx) = unresolved_map.get(&key) {
                                        consumer_import_remap.insert(consumer_imp_func_count, merged_idx);
                                    }
                                }
                                consumer_imp_func_count += 1;
                            }
                        }
                    }
                }
            }
        }

        let consumer_func_base = consumer_func_offsets[ci_ord];
        let parser = Parser::new(0);
        for payload in parser.parse_all(consumer_wasm) {
            if let Ok(Payload::CodeSectionEntry(body)) = payload {
                let mut lgroups: Vec<(u32, wasmparser::ValType)> = Vec::new();
                for l in body.get_locals_reader().map_err(|e| format!("{e}"))? { lgroups.push(l.map_err(|e| format!("{e}"))?); }
                let mut f = Function::new(lgroups.iter().map(|(c, v)| (*c, vt(*v))).collect::<Vec<_>>());
                let mut reader = body.get_operators_reader().map_err(|e| format!("{e}"))?;
                while !reader.eof() {
                    let op = reader.read().map_err(|e| format!("{e}"))?;
                    match op {
                        Operator::Call { function_index } => {
                            if let Some(&new_idx) = consumer_import_remap.get(&function_index) {
                                f.instruction(&I::Call(new_idx));
                            } else if function_index >= consumer_imp_func_count {
                                f.instruction(&I::Call(consumer_func_base + (function_index - consumer_imp_func_count)));
                            } else {
                                f.instruction(&I::Call(function_index));
                            }
                        }
                        _ => { f.instruction(&reencode::RoundtripReencoder.instruction(op).map_err(|e| format!("{e}"))?); }
                    }
                }
                code.function(&f);
            }
        }
    }

    // ── Exports ───��────────────────────────────────────────────────────
    let mut exp = ExportSection::new();
    for e in &orig_exports {
        match e.kind {
            wasmparser::ExternalKind::Func => {
                if let Some(ei) = st_exports.iter().position(|(nm, _)| nm == e.name) {
                    exp.export(e.name, ExportKind::Func, shell_wrappers[ei][0]);
                } else {
                    exp.export(e.name, ExportKind::Func, e.index + fshift);
                }
            }
            wasmparser::ExternalKind::Memory => { exp.export(e.name, ExportKind::Memory, e.index); }
            wasmparser::ExternalKind::Global => { exp.export(e.name, ExportKind::Global, e.index); }
            wasmparser::ExternalKind::Table => { exp.export(e.name, ExportKind::Table, e.index); }
            _ => {}
        }
    }
    // No per-instance __instN exports — consumers call shell wrappers
    // directly via internal import resolution, allowing DCE.
    // Consumer exports
    for (ci_ord, &ci) in consumer_indices.iter().enumerate() {
        let parser = Parser::new(0);
        let mut cifc = 0u32;
        for payload in parser.parse_all(&component.modules[ci].wasm) {
            if let Ok(Payload::ImportSection(reader)) = payload {
                for imp in reader { if let Ok(imp) = imp { if let wasmparser::TypeRef::Func(_) = imp.ty { cifc += 1; } } }
            }
        }
        let parser2 = Parser::new(0);
        for payload in parser2.parse_all(&component.modules[ci].wasm) {
            if let Ok(Payload::ExportSection(reader)) = payload {
                for e in reader {
                    if let Ok(e) = e {
                        if let wasmparser::ExternalKind::Func = e.kind {
                            if e.index >= cifc {
                                exp.export(e.name, ExportKind::Func, consumer_func_offsets[ci_ord] + (e.index - cifc));
                            }
                        }
                        if let wasmparser::ExternalKind::Memory = e.kind {
                            exp.export(&format!("inst{ci_ord}_{}", e.name), ExportKind::Memory, ci_ord as u32);
                        }
                    }
                }
            }
        }
    }

    // ── Assemble ───────────────────────────────────────────────────────
    let mut module = Module::new();
    module.section(&type_sec);

    // Imports: unresolved consumer imports + lib imports + N consumer memories
    {
        let mut imp = ImportSection::new();
        // Unresolved consumer imports first (func indices 0..unresolved_func_count)
        for (i, (mod_name, field, _ti)) in unresolved_imports.iter().enumerate() {
            imp.import(mod_name, field, wasm_encoder::EntityType::Function(unresolved_type_indices[i]));
        }
        for i in &orig_imports {
            match i.ty {
                wasmparser::TypeRef::Func(ti) => { imp.import(i.module, i.name, wasm_encoder::EntityType::Function(ti)); }
                wasmparser::TypeRef::Memory(_) => {}
                wasmparser::TypeRef::Global(gt) => { imp.import(i.module, i.name, wasm_encoder::EntityType::Global(GlobalType { val_type: vt(gt.content_type), mutable: gt.mutable, shared: gt.shared })); }
                _ => {}
            }
        }
        module.section(&imp);
    }

    module.section(&func_sec);
    if !orig_tables.is_empty() {
        let mut t = TableSection::new();
        for tab in &orig_tables { t.table(TableType { element_type: wasm_encoder::RefType::FUNCREF, table64: tab.table64, minimum: tab.initial as u64, maximum: tab.maximum.map(|m| m as u64), shared: false }); }
        module.section(&t);
    }

    // Memories: N copies of each library memory (owned, not imported)
    {
        let mut mem_sec = wasm_encoder::MemorySection::new();
        for mem in &orig_memories {
            for _inst in 0..n {
                mem_sec.memory(MemoryType {
                    minimum: mem.initial, maximum: mem.maximum, memory64: mem.memory64,
                    shared: mem.shared, page_size_log2: mem.page_size_log2,
                });
            }
        }
        module.section(&mem_sec);
    }

    // Globals: mutable × N, immutable × 1, then N immutable instance indices
    {
        let mut g = GlobalSection::new();
        for (gt, init) in &orig_globals {
            let v = vt(gt.content_type); let ce = convert_const_expr(init);
            if gt.mutable { for _ in 0..n { g.global(GlobalType { val_type: v, mutable: true, shared: gt.shared }, &ce); } }
            else { g.global(GlobalType { val_type: v, mutable: false, shared: gt.shared }, &ce); }
        }
        for inst in 0..n { g.global(GlobalType { val_type: ValType::I32, mutable: false, shared: false }, &ConstExpr::i32_const(inst as i32)); }
        module.section(&g);
    }

    module.section(&exp);
    if !orig_elements.is_empty() {
        let mut es = wasm_encoder::ElementSection::new();
        for e in &orig_elements { reencode::RoundtripReencoder.parse_element(&mut es, e.clone()).map_err(|e| format!("{e}"))?; }
        module.section(&es);
    }
    module.section(&code);

    if !orig_data.is_empty() {
        let mut d = DataSection::new();
        for seg in &orig_data {
            match &seg.kind {
                wasmparser::DataKind::Active { memory_index, offset_expr } => {
                    let o = convert_const_expr(offset_expr);
                    for inst in 0..n { d.segment(DataSegment { mode: DataSegmentMode::Active { memory_index: memory_index * n + inst, offset: &o }, data: seg.data.to_vec() }); }
                }
                wasmparser::DataKind::Passive => { d.segment(DataSegment { mode: DataSegmentMode::Passive, data: seg.data.to_vec() }); }
            }
        }
        module.section(&d);
    }

    Ok(module.finish())
}

// ── Helpers ────────────────────────────────────────────────────────────

fn convert_const_expr(expr: &wasmparser::ConstExpr) -> ConstExpr {
    let mut r = expr.get_operators_reader();
    while let Ok(op) = r.read() { match op { Operator::I32Const { value } => return ConstExpr::i32_const(value), Operator::I64Const { value } => return ConstExpr::i64_const(value), Operator::End => break, _ => {} } }
    ConstExpr::i32_const(0)
}

fn mem_op_name(op: &Operator) -> Option<String> {
    mem_op_name_offset(op).map(|(n, _)| n)
}

fn mem_op_name_offset(op: &Operator) -> Option<(String, u64)> {
    match op {
        Operator::I32Load { memarg } => Some(("i32.load".into(), memarg.offset)),
        Operator::I64Load { memarg } => Some(("i64.load".into(), memarg.offset)),
        Operator::F32Load { memarg } => Some(("f32.load".into(), memarg.offset)),
        Operator::F64Load { memarg } => Some(("f64.load".into(), memarg.offset)),
        Operator::I32Load8S { memarg } => Some(("i32.load8_s".into(), memarg.offset)),
        Operator::I32Load8U { memarg } => Some(("i32.load8_u".into(), memarg.offset)),
        Operator::I32Load16S { memarg } => Some(("i32.load16_s".into(), memarg.offset)),
        Operator::I32Load16U { memarg } => Some(("i32.load16_u".into(), memarg.offset)),
        Operator::I64Load8S { memarg } => Some(("i64.load8_s".into(), memarg.offset)),
        Operator::I64Load8U { memarg } => Some(("i64.load8_u".into(), memarg.offset)),
        Operator::I64Load16S { memarg } => Some(("i64.load16_s".into(), memarg.offset)),
        Operator::I64Load16U { memarg } => Some(("i64.load16_u".into(), memarg.offset)),
        Operator::I64Load32S { memarg } => Some(("i64.load32_s".into(), memarg.offset)),
        Operator::I64Load32U { memarg } => Some(("i64.load32_u".into(), memarg.offset)),
        Operator::I32Store { memarg } => Some(("i32.store".into(), memarg.offset)),
        Operator::I64Store { memarg } => Some(("i64.store".into(), memarg.offset)),
        Operator::F32Store { memarg } => Some(("f32.store".into(), memarg.offset)),
        Operator::F64Store { memarg } => Some(("f64.store".into(), memarg.offset)),
        Operator::I32Store8 { memarg } => Some(("i32.store8".into(), memarg.offset)),
        Operator::I32Store16 { memarg } => Some(("i32.store16".into(), memarg.offset)),
        Operator::I64Store8 { memarg } => Some(("i64.store8".into(), memarg.offset)),
        Operator::I64Store16 { memarg } => Some(("i64.store16".into(), memarg.offset)),
        Operator::I64Store32 { memarg } => Some(("i64.store32".into(), memarg.offset)),
        Operator::MemorySize { .. } => Some(("memory.size".into(), 0)),
        Operator::MemoryGrow { .. } => Some(("memory.grow".into(), 0)),
        Operator::MemoryFill { .. } => Some(("memory.fill".into(), 0)),
        Operator::MemoryCopy { .. } => Some(("memory.copy".into(), 0)),
        _ => None,
    }
}

fn mem_op_sig(name: &str) -> (Vec<ValType>, Vec<ValType>) {
    use ValType::*;
    match name {
        "i32.load"|"i32.load8_s"|"i32.load8_u"|"i32.load16_s"|"i32.load16_u" => (vec![I32], vec![I32]),
        "i64.load"|"i64.load8_s"|"i64.load8_u"|"i64.load16_s"|"i64.load16_u"|"i64.load32_s"|"i64.load32_u" => (vec![I32], vec![I64]),
        "f32.load" => (vec![I32], vec![F32]), "f64.load" => (vec![I32], vec![F64]),
        "i32.store"|"i32.store8"|"i32.store16" => (vec![I32, I32], vec![]),
        "i64.store"|"i64.store8"|"i64.store16"|"i64.store32" => (vec![I32, I64], vec![]),
        "f32.store" => (vec![I32, F32], vec![]), "f64.store" => (vec![I32, F64], vec![]),
        "memory.size" => (vec![], vec![I32]), "memory.grow" => (vec![I32], vec![I32]),
        "memory.fill" => (vec![I32, I32, I32], vec![]), "memory.copy" => (vec![I32, I32, I32], vec![]),
        _ => panic!("Unknown: {name}"),
    }
}

use wasm_encoder::Instruction as I;

fn gen_mem_dispatch(n: u32, idx_param: u32, name: &str, orig_param_count: u32) -> Function {
    let mut f = Function::new([]);
    f.instruction(&I::Block(wasm_encoder::BlockType::Empty));
    for _ in 0..n { f.instruction(&I::Block(wasm_encoder::BlockType::Empty)); }
    f.instruction(&I::LocalGet(idx_param));
    let t: Vec<u32> = (0..n).collect();
    f.instruction(&I::BrTable(std::borrow::Cow::Owned(t), n));
    for i in 0..n {
        f.instruction(&I::End);
        for p in 0..orig_param_count { f.instruction(&I::LocalGet(p)); }
        emit_mem_op(&mut f, name, i);
        f.instruction(&I::Return);
    }
    f.instruction(&I::End); f.instruction(&I::Unreachable); f.instruction(&I::End); f
}

fn emit_mem_op(f: &mut Function, name: &str, m: u32) {
    let ma = |a| wasm_encoder::MemArg { offset: 0, align: a, memory_index: m };
    match name {
        "i32.load" => { f.instruction(&I::I32Load(ma(2))); } "i64.load" => { f.instruction(&I::I64Load(ma(3))); }
        "f32.load" => { f.instruction(&I::F32Load(ma(2))); } "f64.load" => { f.instruction(&I::F64Load(ma(3))); }
        "i32.load8_s" => { f.instruction(&I::I32Load8S(ma(0))); } "i32.load8_u" => { f.instruction(&I::I32Load8U(ma(0))); }
        "i32.load16_s" => { f.instruction(&I::I32Load16S(ma(1))); } "i32.load16_u" => { f.instruction(&I::I32Load16U(ma(1))); }
        "i64.load8_s" => { f.instruction(&I::I64Load8S(ma(0))); } "i64.load8_u" => { f.instruction(&I::I64Load8U(ma(0))); }
        "i64.load16_s" => { f.instruction(&I::I64Load16S(ma(1))); } "i64.load16_u" => { f.instruction(&I::I64Load16U(ma(1))); }
        "i64.load32_s" => { f.instruction(&I::I64Load32S(ma(2))); } "i64.load32_u" => { f.instruction(&I::I64Load32U(ma(2))); }
        "i32.store" => { f.instruction(&I::I32Store(ma(2))); } "i64.store" => { f.instruction(&I::I64Store(ma(3))); }
        "f32.store" => { f.instruction(&I::F32Store(ma(2))); } "f64.store" => { f.instruction(&I::F64Store(ma(3))); }
        "i32.store8" => { f.instruction(&I::I32Store8(ma(0))); } "i32.store16" => { f.instruction(&I::I32Store16(ma(1))); }
        "i64.store8" => { f.instruction(&I::I64Store8(ma(0))); } "i64.store16" => { f.instruction(&I::I64Store16(ma(1))); }
        "i64.store32" => { f.instruction(&I::I64Store32(ma(2))); }
        "memory.size" => { f.instruction(&I::MemorySize(m)); } "memory.grow" => { f.instruction(&I::MemoryGrow(m)); }
        "memory.fill" => { f.instruction(&I::MemoryFill(m)); }
        "memory.copy" => { f.instruction(&I::MemoryCopy { src_mem: m, dst_mem: m }); }
        _ => panic!("Unknown: {name}"),
    }
}

fn gen_global_get_dispatch(n: u32, idx_param: u32, base: u32) -> Function {
    let mut f = Function::new([]);
    f.instruction(&I::Block(wasm_encoder::BlockType::Empty));
    for _ in 0..n { f.instruction(&I::Block(wasm_encoder::BlockType::Empty)); }
    f.instruction(&I::LocalGet(idx_param));
    let t: Vec<u32> = (0..n).collect();
    f.instruction(&I::BrTable(std::borrow::Cow::Owned(t), n));
    for i in 0..n { f.instruction(&I::End); f.instruction(&I::GlobalGet(base + i)); f.instruction(&I::Return); }
    f.instruction(&I::End); f.instruction(&I::Unreachable); f.instruction(&I::End); f
}

fn gen_global_set_dispatch(n: u32, idx_param: u32, base: u32) -> Function {
    let mut f = Function::new([]);
    f.instruction(&I::Block(wasm_encoder::BlockType::Empty));
    for _ in 0..n { f.instruction(&I::Block(wasm_encoder::BlockType::Empty)); }
    f.instruction(&I::LocalGet(idx_param));
    let t: Vec<u32> = (0..n).collect();
    f.instruction(&I::BrTable(std::borrow::Cow::Owned(t), n));
    for i in 0..n { f.instruction(&I::End); f.instruction(&I::LocalGet(0)); f.instruction(&I::GlobalSet(base + i)); f.instruction(&I::Return); }
    f.instruction(&I::End); f.instruction(&I::Unreachable); f.instruction(&I::End); f
}
