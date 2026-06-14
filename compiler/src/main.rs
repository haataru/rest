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
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compile .rf file to an executable
    Build {
        input: PathBuf,
        #[arg(short = 'o', long, default_value = "a.out")]
        output: PathBuf,
        #[arg(short = 'O', long, default_value = "none")]
        opt: String,
    },
    /// Compile and run .rf file immediately
    Run {
        input: PathBuf,
        #[arg(short = 'O', long, default_value = "none")]
        opt: String,
    },
    /// Emit LLVM IR (.ll / .bc) instead of a binary
    Llvm {
        input: PathBuf,
        #[arg(short = 'o', long, default_value = "out.ll")]
        output: PathBuf,
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
                let mut lib_path = std::env::current_exe().unwrap();
                lib_path.pop(); // remove `restc`
                lib_path.push("librest_runtime.a");
                if lib_path.exists() {
                    args.push(lib_path.to_string_lossy().into_owned());
                } else {
                    args.push("-lrest_runtime".to_string()); // Fallback if installed
                }
                args.push("-lpthread".to_string());
                args.push("-ldl".to_string());
                args.push("-lutil".to_string());
                args.push("-lrt".to_string());
                args.push("-lm".to_string());
                args.push("-lc".to_string());
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
            let mut lib_path = std::env::current_exe().unwrap();
            lib_path.pop(); // remove `restc`
            lib_path.push("librest_runtime.a");
            if lib_path.exists() {
                args.push(lib_path.to_string_lossy().into_owned());
            } else {
                args.push("-lrest_runtime".to_string()); // Fallback if installed
            }
            args.push("-lpthread".to_string());
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

fn read_source(input: &Path) -> Result<String> {
    if input.extension().is_none_or(|e| e != "rest") {
        anyhow::bail!("input file must have .rest extension");
    }
    if !input.exists() {
        anyhow::bail!("input file not found: {}", input.display());
    }
    std::fs::read_to_string(input).with_context(|| format!("failed to read {}", input.display()))
}

fn compile_to_object(input: &Path, opt: &str) -> Result<(String, PathBuf)> {
    let source = read_source(input)?;
    let level = opt_level(opt)?;
    let o_path = input.with_extension("o");
    driver::run(&source, &o_path, level)?;
    Ok((source, o_path))
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Build { input, output, opt } => {
            let (_source, o_path) = compile_to_object(&input, &opt)?;
            try_linker(&o_path, &output)?;
            let _ = std::fs::remove_file(&o_path);
        }
        Commands::Run { input, opt } => {
            let (_source, o_path) = compile_to_object(&input, &opt)?;
            let exe_path = input.with_extension("out");
            try_linker(&o_path, &exe_path)?;
            let _ = std::fs::remove_file(&o_path);
            let status = std::process::Command::new(&exe_path)
                .status()
                .context("failed to run compiled binary")?;
            let _ = std::fs::remove_file(&exe_path);
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        Commands::Llvm { input, output, opt } => {
            let source = read_source(&input)?;
            let level = opt_level(&opt)?;
            driver::run(&source, &output, level)?;
        }
    }

    Ok(())
}
