use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use inkwell::OptimizationLevel;

use restc::driver;

fn opt_level(s: &str) -> Result<OptimizationLevel> {
    match s {
        "0" | "none" => Ok(OptimizationLevel::None),
        "1" | "less" => Ok(OptimizationLevel::Less),
        "2" | "default" => Ok(OptimizationLevel::Default),
        "3" | "aggressive" => Ok(OptimizationLevel::Aggressive),
        _ => anyhow::bail!("invalid optimization level: {}", s),
    }
}

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Compile .rf file to an executable
    Build {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(short = 'o', long, default_value = "a.out")]
        output: PathBuf,
        #[arg(short = 'O', long, default_value = "none")]
        opt: String,
    },
    /// Compile and run .rf file immediately
    Run {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(short = 'O', long, default_value = "none")]
        opt: String,
    },
    /// Emit LLVM IR (.ll / .bc) instead of a binary
    Llvm {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(short = 'o', long, default_value = "out.ll")]
        output: PathBuf,
        opt: String,
    },
    /// Compile and run the .rest file directly in memory via JIT
    Jit {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(short = 'O', long, default_value = "none")]
        opt: String,
    },
}

fn try_linker(o_path: &Path, exe_path: &Path) -> Result<()> {
    let linkers = ["cc", "gcc", "clang", "ld"];

    for linker in &linkers {
        let mut cmd = std::process::Command::new(linker);
        if *linker == "ld" {
            #[cfg(target_os = "linux")]
            {
                let mut args = vec![
                    "-dynamic-linker".to_string(),
                    "/lib64/ld-linux-x86-64.so.2".to_string(),
                    "-o".to_string(),
                    exe_path.to_string_lossy().into_owned(),
                    o_path.to_string_lossy().into_owned(),
                ];
                args.push("-lpthread".to_string());
                args.push("-ldl".to_string());
                args.push("-lutil".to_string());
                args.push("-lrt".to_string());
                args.push("-lm".to_string());
                args.push("-lc".to_string());
                args.push("-lgcc_s".to_string());
                cmd.args(args);
            }
            #[cfg(target_os = "macos")]
            {
                cmd.args([
                    "-lSystem",
                    "-o",
                    &exe_path.to_string_lossy(),
                    &o_path.to_string_lossy(),
                ]);
            }
            #[cfg(target_os = "windows")]
            {
                cmd.args(["-o", &exe_path.to_string_lossy(), &o_path.to_string_lossy()]);
            }
        } else {
            let mut args = vec![
                "-no-pie".to_string(),
                "-o".to_string(),
                exe_path.to_string_lossy().into_owned(),
                o_path.to_string_lossy().into_owned(),
            ];

            args.push("-ldl".to_string());
            args.push("-lutil".to_string());
            args.push("-lrt".to_string());
            args.push("-lm".to_string());
            cmd.args(args);
        }
        let output = cmd
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output();
        match output {
            Ok(o) if o.status.success() => return Ok(()),
            Ok(o) => {
                let stderr = String::from_utf8(o.stderr)
                    .unwrap_or_else(|e| format!("(linker stderr is not UTF-8: {})", e));
                eprintln!("linker stderr:\n{}", stderr);
            }
            _ => {}
        }
    }

    anyhow::bail!(
        "linker not found

tried: cc, gcc, clang, ld

install one of:
  apt install gcc       (Debian/Ubuntu)
  apt install clang     (Debian/Ubuntu)
  xcode-select --install (macOS)
  pacman -S gcc         (Arch Linux)
  dnf install gcc       (Fedora)

or generate .ll and link manually:
  ref llvm file.rf -o file.ll"
    );
}

// removed read_inputs

fn print_ascii_art() {
    use colored::Colorize;
    let art = r#"
    ____  ___________ ______
   / __ \/ ____/ ___//_  __/
  / /_/ / __/  \__ \  / /   
 / _, _/ /___ ___/ / / /    
/_/ |_/_____//____/ /_/     
"#;
    println!("{}", art.cyan().bold());
    println!("{}", "REST Compiler v0.1.0".green().bold());
    println!("Type `restc --help` for usage.\n");
}

fn handle_error(err: anyhow::Error) {
    use colored::Colorize;
    eprintln!("\n{} {}", "[!] COMPILATION ERROR:".red().bold(), err.to_string().yellow());
    std::process::exit(1);
}

fn main() -> Result<()> {
    use colored::Colorize;
    colored::control::set_override(true);

    let cli = Cli::parse();

    let cmd = match cli.command {
        Some(c) => c,
        None => {
            print_ascii_art();
            use clap::CommandFactory;
            let mut cmd = Cli::command();
            cmd.print_help()?;
            return Ok(());
        }
    };

    match cmd {
        Commands::Build { inputs, output, opt } => {
            let level = opt_level(&opt)?;
            let o_path = output.with_extension("o");
            if let Err(e) = driver::run(&inputs, &o_path, level) {
                handle_error(e);
            }
            if let Err(e) = try_linker(&o_path, &output) {
                handle_error(e);
            }
            println!("{}", format!("[✓] SUCCESS: Binary `{}` compiled successfully!", output.display()).blue().bold());
        }
        Commands::Run { inputs, opt } => {
            let level = opt_level(&opt)?;
            let o_path = PathBuf::from("temp.o");
            let exe_path = PathBuf::from("./temp.out");
            if let Err(e) = driver::run(&inputs, &o_path, level) {
                handle_error(e);
            }
            if let Err(e) = try_linker(&o_path, &exe_path) {
                handle_error(e);
            }
            let _ = std::fs::remove_file(&o_path);
            let status = std::process::Command::new(&exe_path)
                .status()
                .context("failed to run executable");
            let _ = std::fs::remove_file(&exe_path);
            
            match status {
                Ok(s) if !s.success() => std::process::exit(s.code().unwrap_or(1)),
                Err(e) => handle_error(e),
                _ => {}
            }
        }
        Commands::Llvm { inputs, output, opt } => {
            let level = opt_level(&opt)?;
            if let Err(e) = driver::run(&inputs, &output, level) {
                handle_error(e);
            }
            println!("{}", format!("[✓] SUCCESS: LLVM IR written to `{}`!", output.display()).blue().bold());
        }
        Commands::Jit { inputs, opt } => {
            let level = opt_level(&opt)?;
            match driver::run_jit(&inputs, level) {
                Ok(ret) if ret != 0 => std::process::exit(ret),
                Err(e) => handle_error(e),
                _ => {}
            }
        }
    }

    Ok(())
}
