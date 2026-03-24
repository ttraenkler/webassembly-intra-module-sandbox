use wasmparser::{Parser, Payload};

/// A core module extracted from a component.
pub struct CoreModule {
    /// Raw binary wasm bytes for this core module.
    pub wasm: Vec<u8>,
}

/// A core instance instantiation.
pub struct CoreInstance {
    /// Index into `Component::modules` for the module being instantiated.
    pub module_idx: u32,
    /// Arguments: (import_namespace, source_instance_index).
    pub args: Vec<(String, u32)>,
}

/// Parsed component structure.
pub struct Component {
    pub modules: Vec<CoreModule>,
    pub instances: Vec<CoreInstance>,
}

/// Extract core modules and instantiation wiring from a binary component.
pub fn extract_component(wasm: &[u8]) -> Result<Component, String> {
    let parser = Parser::new(0);
    let mut modules = Vec::new();
    let mut instances = Vec::new();

    // Track nesting depth: we only care about the top-level component's
    // direct children, not nested sub-components.
    let mut depth = 0u32;

    for payload in parser.parse_all(wasm) {
        let payload = payload.map_err(|e| format!("Parse error: {e}"))?;

        match &payload {
            Payload::ComponentSection { .. } => {
                depth += 1;
            }
            Payload::ModuleSection { .. } => {
                // ModuleSection increases depth but we also want to capture it
                // at the top level (depth == 0)
            }
            Payload::End { .. } if depth > 0 => {
                depth -= 1;
                continue;
            }
            _ => {}
        }

        // Only process top-level payloads
        if depth > 0 {
            continue;
        }

        match payload {
            Payload::ModuleSection {
                unchecked_range, ..
            } => {
                let module_bytes = wasm[unchecked_range.start..unchecked_range.end].to_vec();
                modules.push(CoreModule { wasm: module_bytes });
            }

            Payload::InstanceSection(reader) => {
                for instance in reader {
                    let instance: wasmparser::Instance =
                        instance.map_err(|e| format!("Core instance section error: {e}"))?;
                    match instance {
                        wasmparser::Instance::Instantiate { module_index, args } => {
                            let mut parsed_args = Vec::new();
                            for arg in args.iter() {
                                match arg.kind {
                                    wasmparser::InstantiationArgKind::Instance => {
                                        parsed_args
                                            .push((arg.name.to_string(), arg.index));
                                    }
                                }
                            }
                            instances.push(CoreInstance {
                                module_idx: module_index,
                                args: parsed_args,
                            });
                        }
                        wasmparser::Instance::FromExports(_) => {
                            // Synthetic instance from exports — skip
                        }
                    }
                }
            }

            _ => {}
        }
    }

    if modules.is_empty() {
        return Err("No core modules found in component".to_string());
    }

    Ok(Component { modules, instances })
}
