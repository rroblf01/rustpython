mod token;
mod ast;
mod parser;
mod bytecode;
mod compiler;
mod object;
mod vm;

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
            let msg = format!("{}", e);
            Err(format!("Traceback (most recent call last):\n  File \"{}\", line ???\n{}\n{}", filename, msg, msg))
        }
    }
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

    vm.run(code).map_err(|e| format!("Runtime error: {}", e))
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() >= 2 {
        // Run a file
        let filename = &args[1];
        match fs::read_to_string(filename) {
            Ok(source) => {
                match run_source(&source, filename) {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("{}", e);
                        std::process::exit(1);
                    }
                }
            }
            Err(e) => {
                eprintln!("Cannot open '{}': {}", filename, e);
                std::process::exit(1);
            }
        }
    } else if args.len() == 1 {
        // REPL
        run_repl();
    }
}
