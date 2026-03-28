//! Specialize merge: analyze the library, specialize state-touching functions
//! per consumer with baked-in memory/global indices, share pure functions.
//! Zero dispatch overhead, no separate rewrite step needed.

use std::collections::{HashMap, HashSet};
use wasm_encoder::{
    CodeSection, ConstExpr, DataSection, DataSegment, DataSegmentMode, ExportKind, ExportSection,
    Function, FunctionSection, GlobalSection, GlobalType, ImportSection, MemoryType,
    Module, TableSection, TableType, TypeSection, ValType,
    reencode::{self, Reencode},
};
use wasmparser::{Operator, Parser, Payload};

use crate::extract::Component;

/// Analyze a module for state-touching functions.
pub(crate) struct LibAnalysis {
    pub(crate) mutable_globals: Vec<u32>,
    pub(crate) state_touching: HashSet<u32>,
    pub(crate) call_graph: HashMap<u32, HashSet<u32>>,
    pub(crate) num_imported_functions: u32,
    pub(crate) num_functions: u32,
    pub(crate) func_type_indices: Vec<u32>,
    pub(crate) _types: Vec<wasmparser::FuncType>,
    pub(crate) exports: Vec<(String, u32)>,
    pub(crate) _memory_count: u32,
}

pub(crate) fn analyze_lib(wasm: &[u8]) -> Result<LibAnalysis, String> {
    let parser = Parser::new(0);
    let mut mutable_globals: Vec<u32> = Vec::new();
    let mut global_count: u32 = 0;
    let mut memory_count: u32 = 0;
    let mut num_imported_functions: u32 = 0;
    let mut func_type_indices: Vec<u32> = Vec::new();
    let mut types: Vec<wasmparser::FuncType> = Vec::new();
    let mut exports: Vec<(String, u32)> = Vec::new();
    let mut direct_state_access: HashSet<u32> = HashSet::new();
    let mut call_graph: HashMap<u32, HashSet<u32>> = HashMap::new();
    let mut defined_func_idx: u32 = 0;

    for payload in parser.parse_all(wasm) {
        let payload = payload.map_err(|e| format!("{e}"))?;
        match payload {
            Payload::TypeSection(r) => { for t in r.into_iter_err_on_gc_types() { types.push(t.map_err(|e| format!("{e}"))?); } }
            Payload::ImportSection(r) => {
                for imp in r {
                    let imp = imp.map_err(|e| format!("{e}"))?;
                    match imp.ty {
                        wasmparser::TypeRef::Func(ti) => { func_type_indices.push(ti); num_imported_functions += 1; }
                        wasmparser::TypeRef::Global(gt) => { if gt.mutable { mutable_globals.push(global_count); } global_count += 1; }
                        wasmparser::TypeRef::Memory(_) => { memory_count += 1; }
                        _ => {}
                    }
                }
            }
            Payload::GlobalSection(r) => {
                for g in r { let g = g.map_err(|e| format!("{e}"))?; if g.ty.mutable { mutable_globals.push(global_count); } global_count += 1; }
            }
            Payload::MemorySection(r) => { for _ in r { memory_count += 1; } }
            Payload::FunctionSection(r) => { for t in r { func_type_indices.push(t.map_err(|e| format!("{e}"))?); } }
            Payload::ExportSection(r) => {
                for e in r { let e = e.map_err(|e| format!("{e}"))?; if let wasmparser::ExternalKind::Func = e.kind { exports.push((e.name.to_string(), e.index)); } }
            }
            Payload::CodeSectionEntry(body) => {
                let fi = num_imported_functions + defined_func_idx;
                let mutable_set: HashSet<u32> = mutable_globals.iter().copied().collect();
                let mut callees: HashSet<u32> = HashSet::new();
                if let Ok(ops) = body.get_operators_reader() {
                    for op in ops {
                        if let Ok(op) = op {
                            match op {
                                Operator::GlobalGet { global_index } | Operator::GlobalSet { global_index } if mutable_set.contains(&global_index) => { direct_state_access.insert(fi); }
                                Operator::I32Load { .. } | Operator::I64Load { .. } | Operator::F32Load { .. } | Operator::F64Load { .. }
                                | Operator::I32Load8S { .. } | Operator::I32Load8U { .. } | Operator::I32Load16S { .. } | Operator::I32Load16U { .. }
                                | Operator::I64Load8S { .. } | Operator::I64Load8U { .. } | Operator::I64Load16S { .. } | Operator::I64Load16U { .. }
                                | Operator::I64Load32S { .. } | Operator::I64Load32U { .. }
                                | Operator::I32Store { .. } | Operator::I64Store { .. } | Operator::F32Store { .. } | Operator::F64Store { .. }
                                | Operator::I32Store8 { .. } | Operator::I32Store16 { .. }
                                | Operator::I64Store8 { .. } | Operator::I64Store16 { .. } | Operator::I64Store32 { .. }
                                | Operator::MemorySize { .. } | Operator::MemoryGrow { .. }
                                | Operator::MemoryFill { .. } | Operator::MemoryCopy { .. } => { direct_state_access.insert(fi); }
                                Operator::Call { function_index } => { callees.insert(function_index); }
                                _ => {}
                            }
                        }
                    }
                }
                call_graph.insert(fi, callees);
                defined_func_idx += 1;
            }
            _ => {}
        }
    }

    let num_functions = num_imported_functions + defined_func_idx;
    let mut state_touching = direct_state_access;
    let mut changed = true;
    while changed {
        changed = false;
        for fi in 0..num_functions {
            if state_touching.contains(&fi) { continue; }
            if let Some(callees) = call_graph.get(&fi) {
                for c in callees {
                    if state_touching.contains(c) { state_touching.insert(fi); changed = true; break; }
                }
            }
        }
    }

    Ok(LibAnalysis { mutable_globals, state_touching, call_graph, num_imported_functions, num_functions, func_type_indices, _types: types, exports, _memory_count: memory_count })
}

/// Trace reachable state-touching functions from a set of entry function indices.
fn reachable_state_touching(starts: &[u32], analysis: &LibAnalysis) -> HashSet<u32> {
    let mut visited = HashSet::new();
    let mut stack: Vec<u32> = starts.to_vec();
    while let Some(f) = stack.pop() {
        if !visited.insert(f) { continue; }
        if let Some(callees) = analysis.call_graph.get(&f) {
            for &c in callees {
                if analysis.state_touching.contains(&c) && !visited.contains(&c) { stack.push(c); }
            }
        }
    }
    visited
}

/// Specialize merge: merge consumers + library with per-instance specialization.
///
/// `lib_idx`: index of the library module in `component.modules`
/// `consumer_indices`: indices of consumer modules
/// `exports_from`: which module's exports to keep (position index)
pub fn specialize_merge(
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

    eprintln!("  Specialize: {} state-touching, {} pure, {} consumers",
        analysis.state_touching.len(),
        analysis.num_functions as usize - analysis.state_touching.len(),
        n);

    // ── Determine which functions each consumer needs ──────────────────
    // For each consumer, find which library exports it imports, then trace reachability.
    let mut per_consumer_needed: Vec<HashSet<u32>> = Vec::new();

    for &ci in consumer_indices {
        let consumer_wasm = &component.modules[ci].wasm;
        let parser = Parser::new(0);
        let mut imported_funcs: Vec<String> = Vec::new();
        for payload in parser.parse_all(consumer_wasm) {
            if let Ok(Payload::ImportSection(reader)) = payload {
                for imp in reader {
                    if let Ok(imp) = imp {
                        if let wasmparser::TypeRef::Func(_) = imp.ty {
                            imported_funcs.push(imp.name.to_string());
                        }
                    }
                }
            }
        }

        // Map imported function names to library export function indices
        let mut entry_funcs: Vec<u32> = Vec::new();
        for name in &imported_funcs {
            for (ename, eidx) in &analysis.exports {
                if ename == name { entry_funcs.push(*eidx); break; }
            }
        }

        let reachable = reachable_state_touching(&entry_funcs, &analysis);
        per_consumer_needed.push(reachable);
    }

    // Union of all needed functions
    let mut all_specialized: HashSet<u32> = HashSet::new();
    for needed in &per_consumer_needed {
        all_specialized.extend(needed);
    }

    let total_copies: u32 = per_consumer_needed.iter().map(|s| s.len() as u32).sum();
    eprintln!("  {} unique state-touching functions, {} total specialized copies",
        all_specialized.len(), total_copies);

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
    let lib_export_names: HashSet<&str> = analysis.exports.iter().map(|(n, _)| n.as_str()).collect();
    let mut unresolved_imports: Vec<(String, String)> = Vec::new();
    let mut unresolved_map: HashMap<(String, String), u32> = HashMap::new();
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
                                unresolved_map.insert(key, idx);
                                unresolved_consumer_types.push(consumer_types[ti as usize].clone());
                                unresolved_imports.push((imp.module.to_string(), imp.name.to_string()));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let fshift = unresolved_imports.len() as u32;

    // ── Global layout: mutable globals × N ─────────────────────────────
    let mut new_global_base: HashMap<u32, u32> = HashMap::new();
    let mut next_global = 0u32;
    for (idx, (gt, _)) in orig_globals.iter().enumerate() {
        new_global_base.insert(idx as u32, next_global);
        next_global += if gt.mutable { n } else { 1 };
    }

    // ── Build output module ────────────────────────────────────────────
    // Types: copy from library
    let mut type_sec = TypeSection::new();
    for ft in &orig_types {
        type_sec.ty().function(ft.params().iter().map(|t| vt(*t)), ft.results().iter().map(|t| vt(*t)));
    }

    // Unresolved consumer import types
    let mut unresolved_type_indices: Vec<u32> = Vec::new();
    for ft in &unresolved_consumer_types {
        let ti = type_sec.len();
        type_sec.ty().function(ft.params().iter().map(|t| vt(*t)), ft.results().iter().map(|t| vt(*t)));
        unresolved_type_indices.push(ti);
    }

    // Also copy types from each consumer
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

    // Functions: original library functions + specialized copies + consumer functions
    let mut func_sec = FunctionSection::new();

    // Library defined functions (pure ones used as-is, state-touching as dead code)
    for &oti in &orig_func_type_indices {
        func_sec.function(oti);
    }
    let lib_defined_count = orig_func_type_indices.len() as u32;
    let mut nw = fshift + analysis.num_imported_functions + lib_defined_count;

    // Specialized copies: per consumer × reachable state-touching functions
    let specialized_list: Vec<u32> = all_specialized.iter().copied().collect();
    let mut specialized_map: HashMap<(u32, u32), u32> = HashMap::new(); // (consumer_idx, lib_func_idx) → new_func_idx

    for (ci_pos, needed) in per_consumer_needed.iter().enumerate() {
        for &fi in &specialized_list {
            if !needed.contains(&fi) { continue; }
            let oti = analysis.func_type_indices[fi as usize];
            func_sec.function(oti);
            specialized_map.insert((ci_pos as u32, fi), nw);
            nw += 1;
        }
    }

    // Shell wrappers: one per consumer per exported state-touching function
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

    // Library original function bodies (pure pass through, state-touching dead code)
    for body in &orig_bodies {
        let mut lgroups: Vec<(u32, wasmparser::ValType)> = Vec::new();
        for l in body.get_locals_reader().map_err(|e| format!("{e}"))? { lgroups.push(l.map_err(|e| format!("{e}"))?); }
        let mut f = Function::new(lgroups.iter().map(|(c, v)| (*c, vt(*v))).collect::<Vec<_>>());
        let mut reader = body.get_operators_reader().map_err(|e| format!("{e}"))?;
        while !reader.eof() {
            let op = reader.read().map_err(|e| format!("{e}"))?;
            match op {
                Operator::Call { function_index } if fshift > 0 => {
                    f.instruction(&I::Call(function_index + fshift));
                }
                _ => {
                    f.instruction(&reencode::RoundtripReencoder.instruction(op).map_err(|e| format!("{e}"))?);
                }
            }
        }
        code.function(&f);
    }

    // Specialized copies
    for (ci_pos, needed) in per_consumer_needed.iter().enumerate() {
        let mem_idx = ci_pos as u32; // this consumer's memory index
        let inst = ci_pos as u32;

        for &fi in &specialized_list {
            if !needed.contains(&fi) { continue; }
            let body_idx = (fi - analysis.num_imported_functions) as usize;
            let body = &orig_bodies[body_idx];

            let mut lgroups: Vec<(u32, wasmparser::ValType)> = Vec::new();
            for l in body.get_locals_reader().map_err(|e| format!("{e}"))? { lgroups.push(l.map_err(|e| format!("{e}"))?); }

            let oti = analysis.func_type_indices[fi as usize];
            let opc = orig_types[oti as usize].params().len() as u32;
            let declared_count: u32 = lgroups.iter().map(|(c, _)| c).sum();

            let mut enc_locals: Vec<(u32, ValType)> = lgroups.iter().map(|(c, v)| (*c, vt(*v))).collect();
            let temp_i32 = opc + declared_count;
            let temp_i64 = temp_i32 + 1;
            let temp_f32 = temp_i64 + 1;
            let temp_f64 = temp_f32 + 1;
            enc_locals.push((1, ValType::I32)); enc_locals.push((1, ValType::I64));
            enc_locals.push((1, ValType::F32)); enc_locals.push((1, ValType::F64));

            let mut f = Function::new(enc_locals);
            let mut reader = body.get_operators_reader().map_err(|e| format!("{e}"))?;

            while !reader.eof() {
                let op = reader.read().map_err(|e| format!("{e}"))?;

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
                    emit_mem_op(&mut f, &name, mem_idx);
                    continue;
                }

                match op {
                    Operator::GlobalGet { global_index } if mutable_set.contains(&global_index) => {
                        f.instruction(&I::GlobalGet(new_global_base[&global_index] + inst));
                    }
                    Operator::GlobalSet { global_index } if mutable_set.contains(&global_index) => {
                        f.instruction(&I::GlobalSet(new_global_base[&global_index] + inst));
                    }
                    Operator::Call { function_index } if all_specialized.contains(&function_index) => {
                        if let Some(&new_idx) = specialized_map.get(&(ci_pos as u32, function_index)) {
                            f.instruction(&I::Call(new_idx));
                        } else {
                            f.instruction(&I::Call(function_index + fshift));
                        }
                    }
                    Operator::Call { function_index } => {
                        f.instruction(&I::Call(function_index + fshift));
                    }
                    _ => {
                        f.instruction(&reencode::RoundtripReencoder.instruction(op).map_err(|e| format!("{e}"))?);
                    }
                }
            }
            code.function(&f);
        }
    }

    // Shell wrappers
    for (_ei, (_, fi)) in st_exports.iter().enumerate() {
        let oti = orig_func_type_indices[(*fi - analysis.num_imported_functions) as usize];
        let ft = &orig_types[oti as usize];
        for ci_pos in 0..n {
            let mut f = Function::new([]);
            for p in 0..ft.params().len() { f.instruction(&I::LocalGet(p as u32)); }
            if let Some(&spec_idx) = specialized_map.get(&(ci_pos, *fi)) {
                f.instruction(&I::Call(spec_idx));
            } else {
                f.instruction(&I::Call(*fi + fshift)); // fallback to original
            }
            f.instruction(&I::End);
            code.function(&f);
        }
    }

    // Consumer function bodies — with import resolution
    for (ci_ord, &ci) in consumer_indices.iter().enumerate() {
        let consumer_wasm = &component.modules[ci].wasm;

        // Build import remap: consumer's imported func idx → merged func idx
        // Consumer imports from "lib" get resolved to shell wrappers for this instance
        let mut consumer_import_remap: HashMap<u32, u32> = HashMap::new();
        let mut consumer_imp_func_count = 0u32;
        {
            let parser = Parser::new(0);
            for payload in parser.parse_all(consumer_wasm) {
                if let Ok(Payload::ImportSection(reader)) = payload {
                    for imp in reader {
                        if let Ok(imp) = imp {
                            if let wasmparser::TypeRef::Func(_) = imp.ty {
                                // Find the library export matching this import name
                                // Check both state-touching (→ shell wrapper) and pure (→ original)
                                if let Some(ei) = st_exports.iter().position(|(n, _)| n == imp.name) {
                                    consumer_import_remap.insert(
                                        consumer_imp_func_count,
                                        shell_wrappers[ei][ci_ord as usize],
                                    );
                                } else if let Some((_, eidx)) = analysis.exports.iter().find(|(n, _)| n == imp.name) {
                                    // Pure function — call directly (shifted)
                                    consumer_import_remap.insert(consumer_imp_func_count, *eidx + fshift);
                                } else {
                                    // Unresolved import (e.g. WASI)
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

        // Count consumer's own defined functions for local call remapping
        let consumer_func_base = consumer_func_offsets[ci_ord];

        let parser = Parser::new(0);
        let mut _consumer_defined_idx = 0u32;
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
                                // Imported function → resolved to shell wrapper or pure function
                                f.instruction(&I::Call(new_idx));
                            } else if function_index >= consumer_imp_func_count {
                                // Local function → remap to merged index
                                let local_idx = function_index - consumer_imp_func_count;
                                f.instruction(&I::Call(consumer_func_base + local_idx));
                            } else {
                                // Unresolved import — pass through (will be invalid but caught by validator)
                                f.instruction(&I::Call(function_index));
                            }
                        }
                        _ => {
                            f.instruction(&reencode::RoundtripReencoder.instruction(op).map_err(|e| format!("{e}"))?);
                        }
                    }
                }
                code.function(&f);
                _consumer_defined_idx += 1;
            }
        }
    }

    // ── Exports ────────────────────────────────────────────────────────
    let mut exp = ExportSection::new();

    // Library exports: state-touching → shell wrapper for instance 0, pure → direct
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
    // directly via internal import resolution. This allows DCE to remove
    // unreachable specialized copies.

    // Consumer exports
    for (ci_ord, &ci) in consumer_indices.iter().enumerate() {
        let parser = Parser::new(0);
        let consumer_func_base = consumer_func_offsets[ci_ord];
        let mut consumer_imp_func_count = 0u32;
        // Count imported funcs to offset defined func indices
        for payload in parser.parse_all(&component.modules[ci].wasm) {
            if let Ok(Payload::ImportSection(reader)) = payload {
                for imp in reader { if let Ok(imp) = imp { if let wasmparser::TypeRef::Func(_) = imp.ty { consumer_imp_func_count += 1; } } }
            }
        }
        let parser2 = Parser::new(0);
        for payload in parser2.parse_all(&component.modules[ci].wasm) {
            if let Ok(Payload::ExportSection(reader)) = payload {
                for e in reader {
                    if let Ok(e) = e {
                        if let wasmparser::ExternalKind::Func = e.kind {
                            if e.index >= consumer_imp_func_count {
                                let new_idx = consumer_func_base + (e.index - consumer_imp_func_count);
                                exp.export(e.name, ExportKind::Func, new_idx);
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

    // Imports: unresolved consumer imports + library imports + consumer memories
    {
        let mut imp = ImportSection::new();
        for (i, (mod_name, field)) in unresolved_imports.iter().enumerate() {
            imp.import(mod_name, field, wasm_encoder::EntityType::Function(unresolved_type_indices[i]));
        }
        for i in &orig_imports {
            match i.ty {
                wasmparser::TypeRef::Func(ti) => { imp.import(i.module, i.name, wasm_encoder::EntityType::Function(ti)); }
                wasmparser::TypeRef::Memory(_) => {} // skip — consumers provide memory
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

    // Globals: mutable × N, immutable × 1
    {
        let mut g = GlobalSection::new();
        for (gt, init) in &orig_globals {
            let v = vt(gt.content_type);
            let ce = convert_const_expr(init);
            if gt.mutable { for _ in 0..n { g.global(GlobalType { val_type: v, mutable: true, shared: gt.shared }, &ce); } }
            else { g.global(GlobalType { val_type: v, mutable: false, shared: gt.shared }, &ce); }
        }
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

use wasm_encoder::Instruction as I;

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
