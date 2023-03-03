use std::env;
use std::fs::File;
use std::io::prelude::*;
use std::io::{self};
use std::path::PathBuf;

#[derive(Debug)]
enum Error {
    Env(env::VarError),
    Io(io::Error),
}

impl From<env::VarError> for Error {
    fn from(error: env::VarError) -> Self {
        Self::Env(error)
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn main() -> Result<(), Error> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=memory.x");

    let out_dir = env::var("OUT_DIR")?;
    let out_dir = PathBuf::from(out_dir);

    let memory_x = include_bytes!("memory.x").as_ref();
    File::create(out_dir.join("memory.x"))?.write_all(memory_x)?;

    // Tell Cargo where to find the file.
    println!("cargo:rustc-link-search={}", out_dir.display());

    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");

    Ok(())
}
