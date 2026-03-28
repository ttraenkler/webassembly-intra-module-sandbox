//! Post-merge isolation verification: checks that every function in the
//! merged module only accesses memory indices it is allowed to.

use std::collections::HashSet;
use wasmparser::{Operator, Parser, Payload};

/// Describes the isolation properties of a merged module.
pub struct MergeManifest {
    /// Number of imported functions (no code bodies to verify).
    pub num_imported_functions: u32,
    /// For each defined function (indexed from 0), the set of memory indices
    /// it may access. None means no memory access allowed (pure, wrapper, etc.).
    pub func_allowed_memories: Vec<Option<HashSet<u32>>>,
}

pub struct Violation {
    pub func_index: u32,
    pub memory_index_used: u32,
    pub op_name: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "func {}: {} accesses memory {}", self.func_index, self.op_name, self.memory_index_used)
    }
}

/// Verify that every function in the merged module only accesses its allowed memories.
pub fn verify_isolation(wasm: &[u8], manifest: &MergeManifest) -> Vec<Violation> {
    let parser = Parser::new(0);
    let mut violations = Vec::new();
    let mut defined_idx = 0u32;

    for payload in parser.parse_all(wasm) {
        let Ok(payload) = payload else { continue };
        if let Payload::CodeSectionEntry(body) = payload {
            let func_idx = manifest.num_imported_functions + defined_idx;
            let allowed = manifest.func_allowed_memories.get(defined_idx as usize);

            if let Ok(ops) = body.get_operators_reader() {
                for op in ops {
                    let Ok(op) = op else { continue };
                    for (mem_idx, name) in extract_memory_indices(&op) {
                        let is_violation = match allowed {
                            Some(Some(set)) => !set.contains(&mem_idx),
                            Some(None) => true, // no memory access allowed
                            None => true,       // function not in manifest
                        };
                        if is_violation {
                            violations.push(Violation {
                                func_index: func_idx,
                                memory_index_used: mem_idx,
                                op_name: name,
                            });
                        }
                    }
                }
            }
            defined_idx += 1;
        }
    }
    violations
}

/// Extract memory indices from a memory instruction (most have one, memory.copy has two).
fn extract_memory_indices(op: &Operator) -> Vec<(u32, String)> {
    match op {
        Operator::I32Load { memarg } => vec![(memarg.memory, "i32.load".into())],
        Operator::I64Load { memarg } => vec![(memarg.memory, "i64.load".into())],
        Operator::F32Load { memarg } => vec![(memarg.memory, "f32.load".into())],
        Operator::F64Load { memarg } => vec![(memarg.memory, "f64.load".into())],
        Operator::I32Load8S { memarg } => vec![(memarg.memory, "i32.load8_s".into())],
        Operator::I32Load8U { memarg } => vec![(memarg.memory, "i32.load8_u".into())],
        Operator::I32Load16S { memarg } => vec![(memarg.memory, "i32.load16_s".into())],
        Operator::I32Load16U { memarg } => vec![(memarg.memory, "i32.load16_u".into())],
        Operator::I64Load8S { memarg } => vec![(memarg.memory, "i64.load8_s".into())],
        Operator::I64Load8U { memarg } => vec![(memarg.memory, "i64.load8_u".into())],
        Operator::I64Load16S { memarg } => vec![(memarg.memory, "i64.load16_s".into())],
        Operator::I64Load16U { memarg } => vec![(memarg.memory, "i64.load16_u".into())],
        Operator::I64Load32S { memarg } => vec![(memarg.memory, "i64.load32_s".into())],
        Operator::I64Load32U { memarg } => vec![(memarg.memory, "i64.load32_u".into())],
        Operator::I32Store { memarg } => vec![(memarg.memory, "i32.store".into())],
        Operator::I64Store { memarg } => vec![(memarg.memory, "i64.store".into())],
        Operator::F32Store { memarg } => vec![(memarg.memory, "f32.store".into())],
        Operator::F64Store { memarg } => vec![(memarg.memory, "f64.store".into())],
        Operator::I32Store8 { memarg } => vec![(memarg.memory, "i32.store8".into())],
        Operator::I32Store16 { memarg } => vec![(memarg.memory, "i32.store16".into())],
        Operator::I64Store8 { memarg } => vec![(memarg.memory, "i64.store8".into())],
        Operator::I64Store16 { memarg } => vec![(memarg.memory, "i64.store16".into())],
        Operator::I64Store32 { memarg } => vec![(memarg.memory, "i64.store32".into())],
        Operator::MemorySize { mem, .. } => vec![(*mem, "memory.size".into())],
        Operator::MemoryGrow { mem, .. } => vec![(*mem, "memory.grow".into())],
        Operator::MemoryFill { mem } => vec![(*mem, "memory.fill".into())],
        Operator::MemoryCopy { dst_mem, src_mem } => vec![
            (*dst_mem, "memory.copy (dst)".into()),
            (*src_mem, "memory.copy (src)".into()),
        ],
        _ => vec![],
    }
}
