//! Native entry point for the independent GPU composition example.

#[cfg(target_os = "macos")]
fn main() {
    rake_gpu_surface::main();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("The GPU surface demo requires macOS.");
    std::process::exit(1);
}
