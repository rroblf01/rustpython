use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

mod token;
mod ast;
mod parser;
mod bytecode;
mod compiler;
mod object;
mod modules;
mod vm;
mod cycle_gc;
#[cfg(feature = "jit")]
mod jit;
mod interner;
#[cfg(feature = "gc")]
mod gc;
#[cfg(feature = "ffi")]
mod ffi_bridge;

use std::env;
use std::fs;
use parser::{Parser, try_parse_as_expression};
use compiler::Compiler;
use vm::VirtualMachine;
use object::{PyObject, ObjectAccess};

/// Exit the process after running atexit handlers.
use object::PyError;

fn print_traceback(vm: &VirtualMachine, e: &PyError, fallback_file: &str) {
    eprintln!("Traceback (most recent call last):");
    if vm.last_traceback.is_empty() {
        let line = vm.last_error_line.map_or("???".to_string(), |l| l.to_string());
        let file = vm.last_error_file.clone().unwrap_or_else(|| fallback_file.to_string());
        eprintln!("  File \"{}\", line {}", file, line);
    } else {
        for (file, line, name) in &vm.last_traceback {
            eprintln!("  File \"{}\", line {}, in {}", file, line, name);
        }
    }
    eprintln!("{}", e);
}

fn call_displayhook(vm: &VirtualMachine, val: &object::PyObjectRef) {
    if let Some(sys_mod) = vm.modules.get("sys") {
        if let Ok(hook) = sys_mod.borrow().get_attribute("displayhook") {
            let hook_borrowed = hook.borrow();
            if let PyObject::BuiltinFunction { func, .. } = &*hook_borrowed {
                let _ = func(&[val.clone()]);
            }
        }
    }
}

fn run_repl_source(vm: &mut VirtualMachine, source: &str) -> Result<object::PyObjectRef, String> {
    // Try expression mode first — preserves the value for sys.displayhook
    if let Ok(program) = try_parse_as_expression(source) {
        let mut compiler = Compiler::new();
        let code = compiler.compile(&program, "<stdin>")
            .map_err(|e| format!("Compile error: {}", e))?;
        return vm.run(code).map_err(|e| format!("{}", e));
    }
    // Fall back to module/statement mode
    let mut parser = Parser::new(source);
    let program = parser.parse_program().map_err(|e| format!("Parse error: {}", e))?;
    let mut compiler = Compiler::new();
    let code = compiler.compile(&program, "<stdin>")
        .map_err(|e| format!("Compile error: {}", e))?;
    vm.run(code).map_err(|e| format!("{}", e))
}

fn should_indent(line: &str) -> bool {
    line.trim().ends_with(':')
}

fn calculate_indent(line: &str, current: usize) -> usize {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        0
    } else if should_indent(line) {
        current + 4
    } else {
        // Count actual leading whitespace of this line, then stay or dedent
        let leading = line.len() - trimmed.len();
        if leading < current {
            leading  // dedent to match this line's actual indent
        } else {
            current
        }
    }
}

fn run_repl() {
    println!("RustPython 0.1.0 - A Python 3 reimplementation in Rust");
    println!("Type 'exit()' or Ctrl-D to quit");
    // NOTE: no trailing blank line — CPython's interactive_python drains
    // the merged stdout+stderr reading 4 bytes at a time looking for the
    // prompt, then falls back to readline() which blocks on the
    // newline-less prompt; a bare blank line before the prompt leaves
    // readline() stuck forever.
    // Flush the banner immediately: stdout to a pipe is block-buffered, and
    // an interactive driver (interactive_python) reads the banner before the
    // prompt.
    use std::io::Write;
    let _ = std::io::stdout().flush();

    let mut vm = VirtualMachine::new();
    let mut rl = rustyline::DefaultEditor::new().map_err(|e| format!("Failed to create editor: {}", e)).unwrap();
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let history_path = std::path::Path::new(&home).join(".rustpython_history");
    let _ = rl.load_history(&history_path);

    // With a piped (non-TTY) stdin — `printf 'print("x")\n' | rustpython`,
    // or `subprocess.Popen(..., stdin=PIPE)` driving the REPL
    // (test_cmd_line_script.py's interactive_python) — rustyline can't read
    // line-by-line and the REPL would block until EOF, hanging any caller
    // that writes a statement then waits for output. Read plain lines from
    // stdin instead in that case.
    use std::io::IsTerminal;
    let piped_stdin = !std::io::stdin().is_terminal();

    let mut source_buf = String::new();
    let mut indent_level = 0;

    loop {
        let prompt = if source_buf.is_empty() { ">>> " } else { "... " };

        let line = if piped_stdin {
            // Real CPython writes REPL prompts to stderr when stdin is not
            // a TTY (so the interactive_python-style drain can find them).
            // The banner has no blank line before the prompt (see the
            // run_repl comment), so the drain's read(4) reaches the prompt
            // directly and never readline()s into it.
            eprint!("{}", prompt);
            let mut buf = String::new();
            use std::io::BufRead;
            match std::io::stdin().lock().read_line(&mut buf) {
                Ok(0) => Err(rustyline::error::ReadlineError::Eof),
                Ok(_) => {
                    if let Some(stripped) = buf.strip_suffix('\n') {
                        buf = stripped.to_string();
                    }
                    if let Some(stripped) = buf.strip_suffix('\r') {
                        buf = stripped.to_string();
                    }
                    Ok(buf)
                }
                Err(e) => Err(rustyline::error::ReadlineError::Io(e)),
            }
        } else if source_buf.is_empty() {
            rl.readline(prompt)
        } else {
            let initial = " ".repeat(indent_level);
            rl.readline_with_initial(prompt, (&initial, ""))
        };

        match line {
            Ok(line) => {
                // readline returns the line without newline

                if !line.is_empty() {
                    let _ = rl.add_history_entry(&line);
                }

                let trimmed = line.trim();

                // Handle exit/quit
                if trimmed == "exit()" || trimmed == "quit()" {
                    break;
                }

                // Empty line while in multi-line mode → force execute
                if trimmed.is_empty() && !source_buf.is_empty() {
                    source_buf.push('\n');
                    match run_repl_source(&mut vm, &source_buf) {
                        Ok(val) => call_displayhook(&vm, &val),
                        Err(e) => eprintln!("{}", e),
                    }
                    // Flush so an interactive driver sees the output
                    // immediately (stdout to a pipe is block-buffered).
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                    source_buf.clear();
                    indent_level = 0;
                    continue;
                }

                // Empty line at top level → skip
                if trimmed.is_empty() {
                    continue;
                }

                source_buf.push_str(&line);
                source_buf.push('\n');

                // Check if the accumulated input is a complete statement
                if is_complete_statement(&source_buf) {
                    match run_repl_source(&mut vm, &source_buf) {
                        Ok(val) => call_displayhook(&vm, &val),
                        Err(e) => eprintln!("{}", e),
                    }
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                    source_buf.clear();
                    indent_level = 0;
                } else {
                    // Still in multi-line mode — update indent for next prompt
                    indent_level = calculate_indent(&line, indent_level);
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                // Ctrl-C: clear buffer and start fresh
                source_buf.clear();
                indent_level = 0;
                println!();
                continue;
            }
            Err(rustyline::error::ReadlineError::Eof) => {
                // Ctrl-D
                println!();
                break;
            }
            Err(e) => {
                eprintln!("Error reading input: {}", e);
                break;
            }
        }
    }

    let _ = rl.save_history(&history_path);
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

// Real CPython lets recursive Python code go ~1000 frames deep
// (`sys.getrecursionlimit()`'s default) before raising a catchable
// `RecursionError`. Each Python-level call here recurses through a large
// chain of actual Rust call frames (`call_function` -> `execute()` ->
// `execute_inner` -> `execute_instruction`'s `CALL` handling ->
// `call_function` -> ...) that, empirically, costs roughly 250KB of REAL
// native stack per level — so the default OS thread stack (a few MB) only
// has room for ~30-40 levels before a hard, uncatchable stack overflow
// aborts the whole process, nowhere near enough for legitimately
// deep-but-not-buggy recursion (tree/graph algorithms, recursive-descent
// parsers, ...). Run everything on a dedicated thread with a much larger
// stack instead, sized to comfortably clear `vm.rs`'s own
// `RecursionError` depth check (see `call_function`'s `self.frames.len()`
// guard) with real headroom to spare.
fn main() {
    let child = std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(real_main)
        .expect("failed to spawn main thread with enlarged stack");
    match child.join() {
        Ok(()) => {}
        Err(_) => std::process::exit(1),
    }
}

fn real_main() {
    if env::var("RPY_DEBUG_SIZES").is_ok() {
        eprintln!("size_of PyObjectRef: {}", std::mem::size_of::<object::PyObjectRef>());
        eprintln!("size_of PyObject: {}", std::mem::size_of::<object::PyObject>());
        eprintln!("size_of PyDict: {}", std::mem::size_of::<object::PyDict>());
        eprintln!("size_of PySet: {}", std::mem::size_of::<object::PySet>());
        eprintln!("size_of vm::Frame: {}", std::mem::size_of::<vm::Frame>());
        eprintln!("size_of SmallVec<[PyObjectRef;8]>: {}", std::mem::size_of::<smallvec::SmallVec<[object::PyObjectRef; 8]>>());
        eprintln!("size_of InternedMap: {}", std::mem::size_of::<crate::interner::InternedMap<object::PyObjectRef>>());
        eprintln!("size_of Option<PyResult>: {}", std::mem::size_of::<Option<Result<object::PyObjectRef, object::PyError>>>());
        eprintln!("size_of Rc<RefCell<HashMap>>: {}", std::mem::size_of::<std::rc::Rc<std::cell::RefCell<std::collections::HashMap<String, object::PyObjectRef>>>>());
        eprintln!("size_of bytecode::CodeObject: {}", std::mem::size_of::<bytecode::CodeObject>());
        eprintln!("size_of bytecode::Instr: {}", std::mem::size_of::<bytecode::Instr>());
        eprintln!("size_of String: {}", std::mem::size_of::<String>());
        eprintln!("size_of HashMap<String,i64> empty: {}", std::mem::size_of::<std::collections::HashMap<String, i64>>());
        eprintln!("size_of Vec<Option<PyObjectRef>>: {}", std::mem::size_of::<Vec<Option<object::PyObjectRef>>>());
        eprintln!("size_of Vec<Option<(u64,PyObjectRef)>>: {}", std::mem::size_of::<Vec<Option<(u64, object::PyObjectRef)>>>());
        eprintln!("size_of ExceptionHandler: {}", std::mem::size_of::<vm::ExceptionHandler>());
        {
            let mut m: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
            m.insert("x".to_string(), 1);
            m.insert("y".to_string(), 2);
            eprintln!("HashMap<String,i64> with 2 entries: capacity={}", m.capacity());
        }
        {
            let mut v: Vec<(String, i64)> = Vec::new();
            v.push(("x".to_string(), 1));
            v.push(("y".to_string(), 2));
            eprintln!("Vec<(String,i64)> with 2 entries: capacity={}", v.capacity());
        }
        std::process::exit(0);
    }
    let raw_args: Vec<String> = env::args().collect();

    // Strip program name
    let args: Vec<String> = raw_args.iter().skip(1).cloned().collect();

    // Handle flags
    let mut i = 0;
    let mut interactive = false;
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
            // `-X <opt>` (implementation-specific extension options, e.g.
            // `-X faulthandler`/`-X dev`), `-I` (isolated mode), and `-E`
            // (ignore `PYTHON*` environment variables) were all entirely
            // unrecognized, immediately erroring out with "unknown option"
            // — real CPython's own `Lib/test/support/script_helper.py`
            // (used pervasively across the CPython test corpus to spawn a
            // subprocess running specific test code in isolation) ALWAYS
            // passes `-X faulthandler`, and conditionally `-I`/`-E` too —
            // so EVERY test using it (a very common pattern: `test_
            // assertion_error_location`, anything using `assert_python_ok`/
            // `run_python_until_end`) failed immediately with exit code 2
            // before ever reaching the actual test code. None of these
            // need real semantics here (no per-flag runtime behavior this
            // interpreter implements differs on) — just needs to not
            // reject them, so `-c`/`-m`/the script filename that follows
            // still gets processed correctly. `i` was previously immutable
            // (this whole loop only ever inspected `args[0]`, since every
            // other arm returns/exits/breaks immediately) — made mutable
            // so multiple flags can now be consumed in sequence.
            "-X" => {
                if i + 1 >= args.len() {
                    eprintln!("rustpython: -X requires an argument");
                    std::process::exit(2);
                }
                i += 2;
                continue;
            }
            "-I" | "-E" => {
                i += 1;
                continue;
            }
            // `-i` — interactive mode: run the script/-c/-m first, then drop
            // into the REPL. Was unrecognized ("unknown option"), which made
            // test_cmd_line_script.py's interactive_python ('-i' + piped
            // stdin) spawn a child that printed the error and exited, then
            // hung forever reading its empty stderr looking for the prompt.
            "-i" => {
                interactive = true;
                i += 1;
                continue;
            }
            // `-W <arg>` (warning-control filter) also takes an argument,
            // but real CPython additionally allows it attached directly
            // (`-Wd`, `-Wonce`, ...) rather than as a separate argv entry —
            // `Lib/test/support/script_helper.py` uses exactly this attached
            // form (`-Wd`), which isn't the standalone literal `"-W"` this
            // match matches on, so it was falling through to the unknown-
            // option catch-all and erroring out before `-X`/the script
            // filename were ever reached.
            "-W" => {
                if i + 1 >= args.len() {
                    eprintln!("rustpython: -W requires an argument");
                    std::process::exit(2);
                }
                i += 2;
                continue;
            }
            s if s.starts_with("-W") && s.len() > 2 => {
                i += 1;
                continue;
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

                if std::env::var("RPY_DEBUG_DIS").is_ok() {
                    for (i, instr) in code_obj.instructions.iter().enumerate() {
                        eprintln!("{:4} {:?} {}", i, instr.op, instr.arg);
                    }
                }
                let mut vm = VirtualMachine::new_with_args(sys_argv);
                match vm.run(code_obj) {
                    Ok(_val) => {
                        crate::modules::run_atexit_handlers(&mut vm);
                    }
                    Err(e) => {
                        if let PyError::SystemExit(exit_code) = &e {
                            crate::modules::run_atexit_handlers(&mut vm);
                            std::process::exit(*exit_code);
                        }
                        print_traceback(&vm, &e, "<string>");
                        crate::modules::run_atexit_handlers(&mut vm);
                        std::process::exit(1);
                    }
                }
                return;
            }
            s if s == "-m" || (s.starts_with("-m") && s.len() > 2) => {
                // Run a module as a script. `-mmodname` (joined) and
                // `-m modname` are both valid.
                let module_name: String = if args[i].len() > 2 {
                    args[i][2..].to_string()
                } else {
                    if i + 1 >= args.len() {
                        eprintln!("rustpython: -m requires an argument");
                        std::process::exit(2);
                    }
                    i += 1;
                    args[i].clone()
                };

                // Build sys.argv for -m mode: `i` points at the module-name
                // token in both the joined (`-mmod`) and separated
                // (`-m mod`) forms (for joined, `i` is the `-mmod` token
                // itself), so the remaining script args start at `i + 1`.
                let mut sys_argv: Vec<String> = vec![module_name.clone()];
                if i + 1 < args.len() {
                    sys_argv.extend_from_slice(&args[i + 1..]);
                }

                // Create VM and try to run the module
                let mut vm = VirtualMachine::new_with_args(sys_argv);

                // Real `python -m mod` runs the module's body with its
                // `__name__` set to `__main__`, so the standard
                // `if __name__ == "__main__":` guard fires. The previous
                // implementation just imported the module and called
                // `.main()` (when present) — wrong for any module that
                // guards on `__name__` (test_quopri's `-mquopri` subprocess,
                // `python -m http.server`, `-m unittest`, ...). Import the
                // module, then re-run its own source into its module dict
                // with `__name__` swapped to `__main__`.
                let script = format!(
                    "import sys\nimport importlib\n_mod = importlib.import_module('{}')\n_src = open(_mod.__file__, 'rb').read()\n_code = compile(_src, _mod.__file__, 'exec')\n_mod.__name__ = '__main__'\nsys.modules['__main__'] = _mod\nexec(_code, _mod.__dict__)\n",
                    module_name.replace("'", "\\'")
                );

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
                    Ok(_val) => {
                        crate::modules::run_atexit_handlers(&mut vm);
                    }
                    Err(e) => {
                        if let PyError::SystemExit(exit_code) = &e {
                            crate::modules::run_atexit_handlers(&mut vm);
                            std::process::exit(*exit_code);
                        }
                        print_traceback(&vm, &e, "<module>");
                        crate::modules::run_atexit_handlers(&mut vm);
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
                // Real CPython sets `__file__` in the running script's own
                // globals to its path — missing entirely here before (only
                // regularly-imported modules got it, via `import_module_
                // from_file`), breaking any top-level script doing
                // `os.path.dirname(__file__)`-style relative-path lookups
                // (real trigger: several of CPython's own `Lib/test/
                // test_*.py` files read `__file__` directly for fixture
                // data next to the script).
                vm.globals.borrow_mut().insert(crate::interner::intern("__file__"), object::py_str(filename));
                match vm.run(code) {
                    Ok(_val) => {
                        crate::modules::run_atexit_handlers(&mut vm);
                    }
                    Err(e) => {
                        if let PyError::SystemExit(exit_code) = &e {
                            crate::modules::run_atexit_handlers(&mut vm);
                            std::process::exit(*exit_code);
                        }
                        print_traceback(&vm, &e, filename);
                        crate::modules::run_atexit_handlers(&mut vm);
                        std::process::exit(1);
                    }
                }
            }
            Err(e) => {
                eprintln!("Cannot open '{}': {}", filename, e);
                std::process::exit(1);
            }
        }
        // `-i`: drop into the REPL after the script finishes.
        if interactive {
            run_repl();
        }
    } else if raw_args.len() == 1 || interactive {
        // REPL (also forced after -i)
        run_repl();
    } else {
        // No file and not REPL (e.g. just flags)
        print_usage();
    }
}
