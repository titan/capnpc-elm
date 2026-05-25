use anyhow::Context;
use std::io::BufRead;
use std::path::PathBuf;
use std::process::{Command, Stdio};

mod binding;
mod capnproto;
mod elm;
mod output;
mod render;
mod type_mapping;

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
/// CodeGeneratorRequest into ParsedSchema.
fn load_rpc_schema() -> anyhow::Result<capnproto::ParsedSchema> {
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

    let schema = capnproto::parse_schema(&message).context("failed to parse rpc.capnp schema")?;

    Ok(schema)
}

/// 第一阶段：从 BufRead 解析 Cap'n Proto CodeGeneratorRequest，返回 ParsedSchema
pub fn parse_request<R: BufRead>(reader: R) -> anyhow::Result<capnproto::ParsedSchema> {
    let message = capnp::serialize::read_message(reader, Default::default())
        .context("Failed to read CodeGeneratorRequest")?;
    capnproto::parse_schema(&message).context("Failed to parse CodeGeneratorRequest")
}

/// 第二阶段：将 ParsedSchema 绑定为 Elm IR 上下文
/// rpc_schema 可选注入；传 None 时若存在 interface 则自动加载
pub fn bind_to_elm(
    schema: &capnproto::ParsedSchema,
    rpc_schema: Option<&capnproto::ParsedSchema>,
) -> anyhow::Result<elm::ElmContext> {
    let mut context = binding::generate_elm_context(&schema.nodes, &schema.requested_files);
    if context.has_interfaces() {
        let rpc = match rpc_schema {
            Some(preloaded) => preloaded,
            None => &load_rpc_schema()?,
        };
        binding::append_rpc_modules(&mut context, &rpc.nodes, &rpc.requested_files);
    }
    Ok(context)
}

/// 第三阶段：将 ElmContext 渲染为 .elm 文件
pub fn render_elm(context: &elm::ElmContext) -> anyhow::Result<()> {
    render::render_elm_modules(context).with_context(|| "Failed to render modules")
}

/// 第三阶段变体：将 ElmContext 渲染到自定义 OutputWriter
pub fn render_elm_to(
    context: &elm::ElmContext,
    writer: &dyn output::OutputWriter,
) -> anyhow::Result<()> {
    render::render_elm_modules_to(context, writer).with_context(|| "Failed to render modules")
}

/// Parse Cap'n Proto CodeGeneratorRequest and generate Elm code
pub fn generate_elm_code<R: BufRead>(reader: R) -> anyhow::Result<()> {
    let schema = parse_request(reader)?;
    let context = bind_to_elm(&schema, None)?;
    render_elm(&context)?;
    Ok(())
}
