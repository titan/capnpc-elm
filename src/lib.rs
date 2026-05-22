use anyhow::Context;
use std::io::{BufRead, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};

mod binding;
mod capnproto;
mod elm;
mod render;

/// Search standard system include paths for capnp/rpc.capnp
fn find_rpc_schema() -> Option<PathBuf> {
    let candidates = [
        "/usr/include/capnp/rpc.capnp",
        "/usr/local/include/capnp/rpc.capnp",
    ];
    for path in &candidates {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Compile rpc.capnp via `capnp compile -o-` and parse the resulting
/// CodeGeneratorRequest into (nodes, requested_files).
fn load_rpc_schema() -> anyhow::Result<(Vec<capnproto::Node>, Vec<capnproto::RequestedFile>)> {
    let rpc_path = find_rpc_schema()
        .context("capnp/rpc.capnp not found in /usr/include or /usr/local/include")?;

    let output = Command::new("capnp")
        .arg("compile")
        .arg("-o-")
        .arg(&rpc_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("failed to execute `capnp compile -o-`")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("capnp compile failed: {}", stderr);
    }

    let mut cursor = std::io::Cursor::new(output.stdout);
    let message = capnp::serialize::read_message(&mut cursor, Default::default())
        .context("failed to parse rpc.capnp CodeGeneratorRequest")?;

    let (nodes, requested_files) =
        capnproto::parse_schema(&message).context("failed to parse rpc.capnp schema")?;

    Ok((nodes, requested_files))
}

/// Parse Cap'n Proto CodeGeneratorRequest and generate Elm code
pub fn generate_elm_code<R: BufRead>(reader: R) -> anyhow::Result<()> {
    // Parse the main request
    let message = capnp::serialize::read_message(reader, Default::default())
        .context("Failed to read CodeGeneratorRequest")?;

    let (nodes, requested_files) =
        crate::capnproto::parse_schema(&message).context("Failed to parse CodeGeneratorRequest")?;

    // Build Elm context from user schema
    let mut context = binding::generate_elm_context(&nodes, &requested_files);

    // If user schema has interfaces, auto-load rpc.capnp and generate RPC type modules
    if context.has_interfaces() {
        let (rpc_nodes, rpc_files) = load_rpc_schema()?;
        binding::append_rpc_modules(&mut context, &rpc_nodes, &rpc_files);
    }

    // Render Elm modules
    render::render_elm_modules(&context).with_context(|| "Failed to render modules")?;

    Ok(())
}
