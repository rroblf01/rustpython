mod token;
mod ast;
mod parser;
mod bytecode;
mod compiler;
mod object;
mod modules;
mod vm;
mod jit;
mod interner;
mod gc;
mod ffi_bridge;
mod sync;

use std::env;
use std::fs;
use std::io::{self, Write, BufRead};
use parser::Parser;
use compiler::Compiler;
use vm::VirtualMachine;
use object::PyError;

fn run_source(source: &str, filename: &str) -> Result<(), String> {
    let mut parser = Parser::new(source);
    let program = parser.parse_program().map_err(|e| format!("Parse error: {}", e))?;

    let mut compiler = Compiler::new();
    let code = compiler.compile(&program, filename).map_err(|e| format!("Compile error: {}", e))?;

    let mut vm = VirtualMachine::new();
    match vm.run(code) {
        Ok(_val) => {
            Ok(())
        }
        Err(e) => {
            if let PyError::SystemExit(code) = &e {
                std::process::exit(*code);
            }
            let line = vm.last_error_line.map_or("???".to_string(), |l| l.to_string());
            let msg = format!("{}", e);
            Err(format!("Traceback (most recent call last):\n  File \"{}\", line {}\n{}{}", filename, line, msg, if msg.is_empty() { String::new() } else { format!("\n{}", msg) }))
        }
    }
}

fn run_source_with_vm(vm: &mut VirtualMachine, source: &str) -> Result<(), String> {
    let mut parser = Parser::new(source);
    let program = parser.parse_program().map_err(|e| format!("Parse error: {}", e))?;

    let mut compiler = Compiler::new();
    let code = compiler.compile(&program, "<string>").map_err(|e| format!("Compile error: {}", e))?;

    vm.run(code).map(|_| ()).map_err(|e| format!("Runtime error: {}", e))
}

fn run_repl() {
    println!("RustPython 0.1.0 - A Python 3 reimplementation in Rust");
    println!("Type 'exit()' or Ctrl-D to quit");
    println!();

    let mut source_buf = String::new();
    let mut vm = VirtualMachine::new();
    let mut history: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    const MAX_HISTORY: usize = 100;

    loop {
        let prompt = if source_buf.is_empty() { ">>> " } else { "... " };
        print!("{}", prompt);
        io::stdout().flush().unwrap();

        let mut line = String::new();
        match io::stdin().lock().read_line(&mut line) {
            Ok(0) => {
                println!();
                break;
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed == "exit()" || trimmed == "quit()" {
                    break;
                }
                if !trimmed.is_empty() {
                    if history.back().map_or(true, |last| last != trimmed) {
                        history.push_back(trimmed.to_string());
                        if history.len() > MAX_HISTORY {
                            history.pop_front();
                        }
                    }
                }
                if trimmed.is_empty() && !source_buf.is_empty() {
                    source_buf.push('\n');
                    match run_source_in_vm(&mut vm, &source_buf, "<stdin>") {
                        Ok(val) => {
                            if !matches!(&*val.borrow(), object::PyObject::None) {
                                println!("{}", val.repr());
                            }
                        }
                        Err(e) => {
                            eprintln!("{}", e);
                        }
                    }
                    source_buf.clear();
                } else {
                    source_buf.push_str(&line);
                    if is_complete_statement(&source_buf) {
                        match run_source_in_vm(&mut vm, &source_buf, "<stdin>") {
                            Ok(val) => {
                                if !matches!(&*val.borrow(), object::PyObject::None) {
                                    println!("{}", val.repr());
                                }
                            }
                            Err(e) => {
                                eprintln!("{}", e);
                            }
                        }
                        source_buf.clear();
                    }
                }
            }
            Err(e) => {
                eprintln!("Error reading input: {}", e);
                break;
            }
        }
    }
}

fn is_complete_statement(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    // Simple heuristic: check if parser succeeds
    let mut parser = Parser::new(s);
    parser.parse_program().is_ok()
}

fn run_source_in_vm(vm: &mut VirtualMachine, source: &str, filename: &str) -> Result<object::PyObjectRef, String> {
    let mut parser = Parser::new(source);
    let program = parser.parse_program().map_err(|e| format!("Parse error: {}", e))?;

    let mut compiler = Compiler::new();
    let code = compiler.compile(&program, filename).map_err(|e| format!("Compile error: {}", e))?;

    vm.run(code).map_err(|e| {
        let line = vm.last_error_line.map_or("???".to_string(), |l| l.to_string());
        format!("Traceback (most recent call last):\n  File \"{}\", line {}\n{}", filename, line, e)
    })
}

fn print_version() {
    println!("RustPython 0.1.0");
    println!("A Python 3 reimplementation in Rust");
}

fn print_usage() {
    print_version();
    println!();
    println!("Usage: rustpython [option] ... [file] [args]");
    println!("Options:");
    println!("  -c <code>        Execute the Python code in <code>");
    println!("  -m <module>      Run library module as a script");
    println!("  --version        Print version and exit");
    println!("  --help           Print this help and exit");
}

fn main() {
    let raw_args: Vec<String> = env::args().collect();

    // Strip program name
    let mut args: Vec<String> = raw_args.iter().skip(1).cloned().collect();

    // Handle flags
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--version" | "-V" => {
                print_version();
                return;
            }
            "--help" | "-h" => {
                print_usage();
                return;
            }
            "-c" => {
                // Execute Python code string
                if i + 1 >= args.len() {
                    eprintln!("rustpython: -c requires an argument");
                    std::process::exit(2);
                }
                let code = args[i + 1].clone();

                // Build sys.argv for -c mode
                let mut sys_argv: Vec<String> = vec!["-c".to_string()];
                // Any remaining args after -c <code> go to sys.argv
                if i + 2 < args.len() {
                    sys_argv.extend_from_slice(&args[i + 2..]);
                }

                // Run the code
                let mut parser = Parser::new(&code);
                let program = match parser.parse_program() {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("Parse error: {}", e);
                        std::process::exit(1);
                    }
                };
                let mut compiler = Compiler::new();
                let code_obj = match compiler.compile(&program, "<string>") {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("Compile error: {}", e);
                        std::process::exit(1);
                    }
                };

                let mut vm = VirtualMachine::new_with_args(sys_argv);
                match vm.run(code_obj) {
                    Ok(_val) => {}
                    Err(e) => {
                        if let PyError::SystemExit(exit_code) = &e {
                            std::process::exit(*exit_code);
                        }
                        let line = vm.last_error_line.map_or("???".to_string(), |l| l.to_string());
                        eprintln!("Traceback (most recent call last):\n  File \"<string>\", line {}\n{}", line, e);
                        std::process::exit(1);
                    }
                }
                return;
            }
            "-m" => {
                // Run a module as a script
                if i + 1 >= args.len() {
                    eprintln!("rustpython: -m requires an argument");
                    std::process::exit(2);
                }
                let module_name = args[i + 1].clone();

                // Build sys.argv for -m mode
                let mut sys_argv: Vec<String> = vec![module_name.clone()];
                if i + 2 < args.len() {
                    sys_argv.extend_from_slice(&args[i + 2..]);
                }

                // Create VM and try to run the module
                let mut vm = VirtualMachine::new_with_args(sys_argv);

                // Create a __main__-like script that imports and runs the module
                let main_script = format!(
                    "import runpy\nrunpy._run_module_as_main('{}', alter_argv=True)\n",
                    module_name.replace("'", "\\'")
                );

                // If runpy isn't available, try simpler approach:
                // import the module and call its __main__.py equivalent
                let alt_script = format!(
                    "import {} as _runmod\nif hasattr(_runmod, 'main'):\n    _runmod.main()\n",
                    module_name
                );

                // First try the simple approach: just import and check __name__
                let script = format!(
                    "import sys\nimport {0}\nsys.modules['__main__'] = sys.modules['{0}']\n", module_name
                ) + "if hasattr(sys.modules['__main__'], 'main'):\n"
                + "    sys.modules['__main__'].main()\n";

                let mut parser = Parser::new(&script);
                let program = match parser.parse_program() {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("Error loading module '{}': {}", module_name, e);
                        std::process::exit(1);
                    }
                };
                let mut compiler = Compiler::new();
                let code_obj = match compiler.compile(&program, "<module>") {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("Error compiling module '{}': {}", module_name, e);
                        std::process::exit(1);
                    }
                };

                match vm.run(code_obj) {
                    Ok(_val) => {}
                    Err(e) => {
                        if let PyError::SystemExit(exit_code) = &e {
                            std::process::exit(*exit_code);
                        }
                        let line = vm.last_error_line.map_or("???".to_string(), |l| l.to_string());
                        eprintln!("Traceback (most recent call last):\n  File \"<module>\", line {}\n{}", line, e);
                        std::process::exit(1);
                    }
                }
                return;
            }
            _ => {
                // First non-flag argument is the filename (or -c/-m)
                if !args[i].starts_with('-') {
                    break;
                }
                // Unknown flag but doesn't start with -? shouldn't happen
                if args[i].starts_with("--") || args[i].starts_with('-') && args[i].len() > 1 {
                    eprintln!("rustpython: unknown option '{}'", args[i]);
                    std::process::exit(2);
                }
                break;
            }
        }
        i += 1;
    }

    // Get remaining args (file + script args)
    let script_args: Vec<String> = if i < args.len() {
        args[i..].to_vec()
    } else {
        vec![]
    };

    if !script_args.is_empty() {
        // Run a file
        let filename = &script_args[0];
        // sys.argv = [filename, ...args]
        let sys_argv = script_args.clone();

        match fs::read_to_string(filename) {
            Ok(source) => {
                let mut parser = Parser::new(&source);
                let program = match parser.parse_program() {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("Parse error in '{}': {}", filename, e);
                        std::process::exit(1);
                    }
                };
                let mut compiler = Compiler::new();
                let code = match compiler.compile(&program, filename) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("Compile error in '{}': {}", filename, e);
                        std::process::exit(1);
                    }
                };

                let mut vm = VirtualMachine::new_with_args(sys_argv);
                match vm.run(code) {
                    Ok(_val) => {}
                    Err(e) => {
                        if let PyError::SystemExit(exit_code) = &e {
                            std::process::exit(*exit_code);
                        }
                        let line = vm.last_error_line.map_or("???".to_string(), |l| l.to_string());
                        eprintln!("Traceback (most recent call last):\n  File \"{}\", line {}\n{}", filename, line, e);
                        std::process::exit(1);
                    }
                }
            }
            Err(e) => {
                eprintln!("Cannot open '{}': {}", filename, e);
                std::process::exit(1);
            }
        }
    } else if raw_args.len() == 1 {
        // REPL
        run_repl();
    } else {
        // No file and not REPL (e.g. just flags)
        print_usage();
    }
}
