use std::env;
use std::fs;
use std::io::Write;
use std::process;

mod dispatch;
mod extract;
mod merge;
mod specialize;

fn usage() -> ! {
    eprintln!("wasm-merge — shared-nothing multi-memory module merger");
    eprintln!();
    eprintln!("USAGE:");
    eprintln!("  # Merge standalone modules");
    eprintln!("  wasm-merge <file.wasm>=<label> [<file.wasm>=<label> ...] -o out.wasm");
    eprintln!();
    eprintln!("  # Merge from a binary component (auto-detected with single file)");
    eprintln!("  wasm-merge <component.wasm> -o out.wasm");
    eprintln!();
    eprintln!("Each module keeps its own linear memory (multi-memory).");
    eprintln!("Cross-module imports are resolved by matching labels to export namespaces.");
    eprintln!("Output is written to stdout if -o is not specified.");
    process::exit(1);
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        usage();
    }

    // Parse -o flag
    let out_index = args.iter().position(|a| a == "-o");
    let out_file = out_index.map(|i| {
        args.get(i + 1)
            .unwrap_or_else(|| {
                eprintln!("Error: -o requires a filename");
                process::exit(1);
            })
            .as_str()
    });

    // Determine mode:
    //   --component flag → component mode
    //   Single .wasm arg without =label → auto component mode
    //   file.wasm=label args → merge mode
    let explicit_component = args.iter().any(|a| a == "--component");

    let positional_args: Vec<&String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| {
            !a.starts_with('-')
                && out_index.map_or(true, |oi| *i != oi + 1)
        })
        .map(|(_, a)| a)
        .collect();

    let auto_component = !explicit_component
        && positional_args.len() == 1
        && !positional_args[0].contains('=');

    let merged = if explicit_component || auto_component {
        run_component_mode(&args, out_index)
    } else {
        run_merge_mode(&args, out_index)
    };

    // Write output
    if let Some(path) = out_file {
        fs::write(path, &merged).unwrap_or_else(|e| {
            eprintln!("Error writing {path}: {e}");
            process::exit(1);
        });
        eprintln!("  ✓ wrote {path}");
    } else {
        std::io::stdout().lock().write_all(&merged).unwrap_or_else(|e| {
            eprintln!("Error writing stdout: {e}");
            process::exit(1);
        });
    }
}

/// Merge standalone .wasm modules: `wasm-merge b.wasm=b a.wasm=a -o out.wasm`
fn run_merge_mode(args: &[String], _out_index: Option<usize>) -> Vec<u8> {
    // Collect file=label pairs and flags
    let mut inputs: Vec<(String, String)> = Vec::new();
    let mut exports_from: Option<String> = None;
    let mut do_specialize = false;
    let mut do_dispatch = false;
    let mut lib_label: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "-o" { i += 2; continue; }
        if arg == "--exports-from" {
            i += 1;
            exports_from = Some(args[i].clone());
            i += 1;
            continue;
        }
        if arg == "--specialize" {
            do_specialize = true;
            i += 1;
            continue;
        }
        if arg == "--dispatch" {
            do_dispatch = true;
            i += 1;
            continue;
        }
        if arg == "--lib" {
            i += 1;
            lib_label = Some(args[i].clone());
            i += 1;
            continue;
        }
        if arg.starts_with('-') { i += 1; continue; }

        if let Some((file, label)) = arg.split_once('=') {
            inputs.push((file.to_string(), label.to_string()));
        } else {
            eprintln!("Error: expected <file.wasm>=<label>, got: {arg}");
            process::exit(1);
        }
        i += 1;
    }

    if inputs.is_empty() {
        eprintln!("Error: no input modules specified");
        usage();
    }

    // Read all modules
    let modules: Vec<extract::CoreModule> = inputs
        .iter()
        .map(|(file, _)| {
            let wasm = fs::read(file).unwrap_or_else(|e| {
                eprintln!("Error reading {file}: {e}");
                process::exit(1);
            });
            extract::CoreModule { wasm }
        })
        .collect();

    // Build instances from the import/export wiring.
    // Scan each module's imports to find which label they reference,
    // then match that to the module with that label.
    let label_to_idx: std::collections::HashMap<String, usize> = inputs
        .iter()
        .enumerate()
        .map(|(i, (_, label))| (label.clone(), i))
        .collect();

    let mut instances: Vec<extract::CoreInstance> = Vec::new();

    for (i, (_file, _label)) in inputs.iter().enumerate() {
        let mut args_for_instance: Vec<(String, u32)> = Vec::new();

        // Scan this module's imports to find which namespaces it imports from
        let parser = wasmparser::Parser::new(0);
        let mut import_namespaces = std::collections::HashSet::new();
        for payload in parser.parse_all(&modules[i].wasm) {
            if let Ok(wasmparser::Payload::ImportSection(reader)) = payload {
                for imp in reader {
                    if let Ok(imp) = imp {
                        import_namespaces.insert(imp.module.to_string());
                    }
                }
            }
        }

        // Match import namespaces to module labels
        for ns in &import_namespaces {
            if let Some(&dep_idx) = label_to_idx.get(ns) {
                // Find or create the instance index for the dependency
                let dep_instance_idx = instances
                    .iter()
                    .position(|inst| inst.module_idx == dep_idx as u32)
                    .unwrap_or_else(|| {
                        // Dependency hasn't been instantiated yet — add it
                        let idx = instances.len();
                        instances.push(extract::CoreInstance {
                            module_idx: dep_idx as u32,
                            args: Vec::new(),
                        });
                        idx
                    });
                args_for_instance.push((ns.clone(), dep_instance_idx as u32));
            }
        }

        // Check if this module already has an instance entry (added as a dependency)
        if let Some(existing) = instances.iter_mut().find(|inst| inst.module_idx == i as u32) {
            existing.args = args_for_instance;
        } else {
            instances.push(extract::CoreInstance {
                module_idx: i as u32,
                args: args_for_instance,
            });
        }
    }

    eprintln!(
        "Merging {} module(s): {}",
        inputs.len(),
        inputs
            .iter()
            .map(|(f, l)| format!("{f} ({l})"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let component = extract::Component { modules, instances };

    if do_specialize || do_dispatch {
        let mode_name = if do_specialize { "specialize" } else { "dispatch" };
        let lib_label = lib_label.unwrap_or_else(|| {
            eprintln!("Error: --{mode_name} requires --lib <label>");
            process::exit(1);
        });
        let lib_idx = label_to_idx.get(&lib_label).copied().unwrap_or_else(|| {
            eprintln!("Error: --lib {lib_label} not found in inputs");
            process::exit(1);
        });
        let consumer_indices: Vec<usize> = (0..inputs.len()).filter(|&i| i != lib_idx).collect();

        eprintln!("{mode_name}: lib={lib_label} (module {lib_idx}), {} consumers", consumer_indices.len());

        if do_specialize {
            specialize::specialize_merge(
                &component,
                lib_idx,
                &consumer_indices,
                exports_from.as_deref().and_then(|s| s.parse::<usize>().ok()),
            ).unwrap_or_else(|e| {
                eprintln!("Specialize error: {e}");
                process::exit(1);
            })
        } else {
            dispatch::dispatch_merge(
                &component,
                lib_idx,
                &consumer_indices,
                exports_from.as_deref().and_then(|s| s.parse::<usize>().ok()),
            ).unwrap_or_else(|e| {
                eprintln!("Dispatch error: {e}");
                process::exit(1);
            })
        }
    } else {
        merge::merge(&component, exports_from.as_deref()).unwrap_or_else(|e| {
            eprintln!("Merge error: {e}");
            process::exit(1);
        })
    }
}

/// Merge from a binary component: `wasm-merge --component component.wasm -o out.wasm`
fn run_component_mode(args: &[String], out_index: Option<usize>) -> Vec<u8> {
    // Find the component file (first non-flag arg that isn't --component or -o's value)
    let input_file = args
        .iter()
        .enumerate()
        .find(|(i, a)| {
            !a.starts_with('-')
                && *a != "--component"
                && out_index.map_or(true, |oi| *i != oi + 1)
        })
        .map(|(_, a)| a.as_str());

    let Some(input_file) = input_file else {
        eprintln!("Error: no component file specified");
        usage();
    };

    let wasm = fs::read(input_file).unwrap_or_else(|e| {
        eprintln!("Error reading {input_file}: {e}");
        process::exit(1);
    });

    let component = extract::extract_component(&wasm).unwrap_or_else(|e| {
        eprintln!("Extract error: {e}");
        process::exit(1);
    });

    eprintln!(
        "Found {} core module(s), {} core instance(s)",
        component.modules.len(),
        component.instances.len()
    );

    for (i, inst) in component.instances.iter().enumerate() {
        let deps: Vec<String> = inst
            .args
            .iter()
            .map(|(ns, src_idx)| format!("\"{ns}\" <- instance {src_idx}"))
            .collect();
        eprintln!(
            "  instance {i}: module {} {}",
            inst.module_idx,
            if deps.is_empty() {
                String::new()
            } else {
                format!("({})", deps.join(", "))
            }
        );
    }

    let merged = merge::merge(&component, None).unwrap_or_else(|e| {
        eprintln!("Merge error: {e}");
        process::exit(1);
    });

    eprintln!("  ✓ merged ({} bytes)", merged.len());
    merged
}
