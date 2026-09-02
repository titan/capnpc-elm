//! 单元测试：内存构造 CodeGeneratorRequest → 三阶段管线 → MemoryWriter 断言。
//!
//! 不写文件、不调 elm-format（AGENTS.md 推荐路线）。
//! 请求消息用 capnp crate 自带的 schema_capnp builder 类型手工拼装，
//! parse 侧实际读取的字段集合见 src/capnproto.rs::parse_node。

use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;

use capnp::message;
use capnp::schema_capnp::code_generator_request;
use capnp::schema_capnp::{field, node, type_};

use crate::output::MemoryWriter;
use crate::{bind_to_elm, parse_request, render_elm_to};

const FILE_ID: u64 = 1;

// ── 请求构造 ──────────────────────────────────────────────

fn build_message(f: impl FnOnce(code_generator_request::Builder<'_>)) -> Vec<u8> {
    let mut msg = message::Builder::new_default();
    let mut req = msg.init_root::<code_generator_request::Builder>();
    f(req.reborrow());
    let mut buf = Vec::new();
    capnp::serialize::write_message(&mut buf, &msg).expect("serialize request");
    buf
}

fn init_file_node(mut n: node::Builder<'_>) {
    n.set_id(FILE_ID);
    n.set_scope_id(0);
    n.set_display_name("/t.capnp:file");
    n.set_file(());
}

/// 无字段空 struct（interface 方法的隐式参数/结果结构体）
fn empty_struct(mut n: node::Builder<'_>, id: u64, display: &str, scope: u64) {
    n.set_id(id);
    n.set_scope_id(scope);
    n.set_display_name(display);
    let mut s = n.init_struct();
    s.set_data_word_count(0);
    s.set_pointer_count(0);
    s.set_discriminant_offset(0);
    s.init_fields(0);
}

/// 普通 slot 字段；discriminant 显式置 0xFFFF（=非 union 成员）
fn add_slot(
    mut f: field::Builder<'_>,
    name: &str,
    offset: u32,
    set_ty: impl FnOnce(type_::Builder<'_>),
    int32_default: Option<i32>,
) {
    f.set_name(name);
    f.set_discriminant_value(0xffff);
    let mut slot = f.init_slot();
    slot.set_offset(offset);
    set_ty(slot.reborrow().init_type());
    if let Some(d) = int32_default {
        slot.set_had_explicit_default(true);
        slot.reborrow().init_default_value().set_int32(d);
    }
}

fn set_single_requested_file(req: code_generator_request::Builder<'_>) {
    let mut rf = req.init_requested_files(1);
    let mut f0 = rf.reborrow().get(0);
    f0.set_id(FILE_ID);
    f0.set_filename("t.capnp");
}

// ── 场景 ──────────────────────────────────────────────────

/// struct Point { x @0 :Int32; y @1 :UInt32; }
fn point_request() -> Vec<u8> {
    build_message(|mut req| {
        {
            let mut nodes = req.reborrow().init_nodes(2);
            init_file_node(nodes.reborrow().get(0));
            let mut n = nodes.reborrow().get(1);
            n.set_id(2);
            n.set_scope_id(FILE_ID);
            n.set_display_name("/t.capnp:Point");
            let mut s = n.init_struct();
            s.set_data_word_count(1);
            s.set_pointer_count(0);
            s.set_discriminant_offset(0);
            let mut fields = s.init_fields(2);
            add_slot(fields.reborrow().get(0), "x", 0, |mut t| {
                t.set_int32(());
            }, None);
            add_slot(fields.reborrow().get(1), "y", 1, |mut t| {
                t.set_uint32(());
            }, None);
        }
        set_single_requested_file(req);
    })
}

/// struct Wide { big @0 :Int64; count @1 :UInt32; }
fn wide_request() -> Vec<u8> {
    build_message(|mut req| {
        {
            let mut nodes = req.reborrow().init_nodes(2);
            init_file_node(nodes.reborrow().get(0));
            let mut n = nodes.reborrow().get(1);
            n.set_id(2);
            n.set_scope_id(FILE_ID);
            n.set_display_name("/t.capnp:Wide");
            let mut s = n.init_struct();
            s.set_data_word_count(2);
            s.set_pointer_count(0);
            s.set_discriminant_offset(0);
            let mut fields = s.init_fields(2);
            add_slot(fields.reborrow().get(0), "big", 0, |mut t| {
                t.set_int64(());
            }, None);
            add_slot(fields.reborrow().get(1), "count", 2, |mut t| {
                t.set_uint32(());
            }, None);
        }
        set_single_requested_file(req);
    })
}

/// enum Status { on @0; off @1; }
fn status_request() -> Vec<u8> {
    build_message(|mut req| {
        {
            let mut nodes = req.reborrow().init_nodes(2);
            init_file_node(nodes.reborrow().get(0));
            let mut n = nodes.reborrow().get(1);
            n.set_id(3);
            n.set_scope_id(FILE_ID);
            n.set_display_name("/t.capnp:Status");
            let e = n.init_enum();
            let mut enumerants = e.init_enumerants(2);
            let mut v = enumerants.reborrow().get(0);
            v.set_name("on");
            v.set_code_order(0);
            let mut v = enumerants.reborrow().get(1);
            v.set_name("off");
            v.set_code_order(1);
        }
        set_single_requested_file(req);
    })
}

/// struct Level { level @0 :Int32 = 5; }
fn level_request() -> Vec<u8> {
    build_message(|mut req| {
        {
            let mut nodes = req.reborrow().init_nodes(2);
            init_file_node(nodes.reborrow().get(0));
            let mut n = nodes.reborrow().get(1);
            n.set_id(2);
            n.set_scope_id(FILE_ID);
            n.set_display_name("/t.capnp:Level");
            let mut s = n.init_struct();
            s.set_data_word_count(1);
            s.set_pointer_count(0);
            s.set_discriminant_offset(0);
            let mut fields = s.init_fields(1);
            add_slot(fields.reborrow().get(0), "level", 0, |mut t| {
                t.set_int32(());
            }, Some(5));
        }
        set_single_requested_file(req);
    })
}

/// struct Machine {
///   power @0 :UInt64;
///   mode :union { on @2 :Void; off @3 :Void; }
/// }
/// 命名（内嵌）union：判别式偏移挂在嵌套 group 节点上（此处 =3，即字节 6），
/// 镜像 rpc.capnp Call/Disembargo 的形态。
fn machine_request() -> Vec<u8> {
    build_message(|mut req| {
        {
            let mut nodes = req.reborrow().init_nodes(3);
            init_file_node(nodes.reborrow().get(0));

            let mut n = nodes.reborrow().get(1);
            n.set_id(200);
            n.set_scope_id(FILE_ID);
            n.set_display_name("/t.capnp:Machine");
            let mut s = n.init_struct();
            s.set_data_word_count(1);
            s.set_pointer_count(0);
            s.set_discriminant_offset(0);
            let mut fields = s.init_fields(2);
            add_slot(fields.reborrow().get(0), "power", 0, |mut t| {
                t.set_uint64(());
            }, None);
            let mut f = fields.reborrow().get(1);
            f.set_name("mode");
            f.set_discriminant_value(0xffff);
            f.init_group().set_type_id(201);

            let mut n = nodes.reborrow().get(2);
            n.set_id(201);
            n.set_scope_id(200);
            n.set_display_name("/t.capnp:Machine.mode");
            let mut s = n.init_struct();
            s.set_is_group(true);
            s.set_data_word_count(1);
            s.set_pointer_count(0);
            s.set_discriminant_offset(3);
            let mut fields = s.init_fields(2);
            let mut f = fields.reborrow().get(0);
            f.set_name("on");
            f.set_discriminant_value(0);
            let mut slot = f.init_slot();
            slot.set_offset(0);
            slot.reborrow().init_type().set_void(());
            let mut f = fields.reborrow().get(1);
            f.set_name("off");
            f.set_discriminant_value(1);
            let mut slot = f.init_slot();
            slot.set_offset(0);
            slot.reborrow().init_type().set_void(());
        }
        set_single_requested_file(req);
    })
}

const IFACE_ID: u64 = 5;
const PARAM_ID: u64 = 10;
const RESULT_ID: u64 = 11;
const HOLDER_ID: u64 = 20;

/// interface Ping { ping @0 () -> (); }
/// struct Holder { cb1 @0 :Ping; cb2 @1 :Ping; }
/// 会触发 bind_to_elm 自动加载系统 rpc.capnp（依赖 capnp CLI）。
fn interface_request() -> Vec<u8> {
    build_message(|mut req| {
        {
            let mut nodes = req.reborrow().init_nodes(5);
            init_file_node(nodes.reborrow().get(0));

            let mut n = nodes.reborrow().get(1);
            n.set_id(IFACE_ID);
            n.set_scope_id(FILE_ID);
            n.set_display_name("/t.capnp:Ping");
            let i = n.init_interface();
            let mut methods = i.init_methods(1);
            let mut m = methods.reborrow().get(0);
            m.set_name("ping");
            m.set_code_order(0);
            m.set_param_struct_type(PARAM_ID);
            m.set_result_struct_type(RESULT_ID);

            empty_struct(
                nodes.reborrow().get(2),
                PARAM_ID,
                "/t.capnp:Ping.ping_params",
                IFACE_ID,
            );
            empty_struct(
                nodes.reborrow().get(3),
                RESULT_ID,
                "/t.capnp:Ping.ping_results",
                IFACE_ID,
            );

            let mut n = nodes.reborrow().get(4);
            n.set_id(HOLDER_ID);
            n.set_scope_id(FILE_ID);
            n.set_display_name("/t.capnp:Holder");
            let mut s = n.init_struct();
            s.set_data_word_count(0);
            s.set_pointer_count(2);
            s.set_discriminant_offset(0);
            let mut fields = s.init_fields(2);
            for (i, name) in ["cb1", "cb2"].iter().enumerate() {
                let mut f = fields.reborrow().get(i as u32);
                f.set_name(name);
                f.set_discriminant_value(0xffff);
                let mut slot = f.init_slot();
                slot.set_offset(i as u32);
                let ty = slot.reborrow().init_type();
                let mut iv = ty.init_interface();
                iv.set_type_id(IFACE_ID);
            }
        }
        set_single_requested_file(req);
    })
}

// ── 渲染辅助 ──────────────────────────────────────────────

fn render_request(bytes: Vec<u8>) -> HashMap<PathBuf, String> {
    let schema = parse_request(Cursor::new(bytes)).expect("parse_request failed");
    let ctx = bind_to_elm(&schema, None).expect("bind_to_elm failed");
    let writer = MemoryWriter::new();
    render_elm_to(&ctx, &writer).expect("render_elm_to failed");
    writer.get_all()
}

/// 按 Path 后缀（组件匹配）取唯一产物，多余/缺失都算失败
fn find<'a>(out: &'a HashMap<PathBuf, String>, suffix: &str) -> &'a str {
    let hits: Vec<&PathBuf> = out.keys().filter(|p| p.ends_with(suffix)).collect();
    assert_eq!(
        hits.len(),
        1,
        "expected exactly one output ending in {suffix:?}, got {hits:?} (all: {:?})",
        out.keys().collect::<Vec<_>>()
    );
    &out[hits[0]]
}

// ── 测试 ──────────────────────────────────────────────────

/// 基础 struct：模块渲染成功，含访问器与编解码器；无 interface 字段则不导出 encodeWithCaps
#[test]
fn struct_module_renders_codec_and_accessors() {
    let out = render_request(point_request());
    let m = find(&out, "Point.elm");
    assert!(m.contains("getX"), "missing accessor getX:\n{m}");
    assert!(m.contains("getY"), "missing accessor getY:\n{m}");
    assert!(m.contains("encode"), "missing encoder:\n{m}");
    assert!(m.contains("decode"), "missing decoder:\n{m}");
    assert!(
        !m.contains("encodeWithCaps"),
        "plain struct must not export encodeWithCaps:\n{m}"
    );
}

/// Int64 → Capnproto.Word64；UInt → Elm Int（writeUInt32）
#[test]
fn int64_maps_to_word64_and_uint_to_int() {
    let out = render_request(wide_request());
    let m = find(&out, "Wide.elm");
    assert!(
        m.contains("Capnproto.Word64"),
        "Int64 field must map to Capnproto.Word64:\n{m}"
    );
    assert!(
        m.contains("writeUInt32"),
        "UInt32 field must use writeUInt32:\n{m}"
    );
}

/// enum：变体名渲染进自定义类型
#[test]
fn enum_module_renders_variants() {
    let out = render_request(status_request());
    let m = find(&out, "Status.elm");
    assert!(m.contains("type Entity"), "missing type Entity:\n{m}");
    assert!(m.contains("On"), "missing variant On:\n{m}");
    assert!(m.contains("Off"), "missing variant Off:\n{m}");
}

/// 非零默认值 → encoder 生成 XOR mask（wire format 要求）
#[test]
fn nonzero_default_generates_xor_mask() {
    let out = render_request(level_request());
    let m = find(&out, "Level.elm");
    assert!(
        m.contains("Bitwise.xor entity.level 5"),
        "default 5 must XOR-mask on encode:\n{m}"
    );
}

/// 命名 union 的判别式偏移必须来自嵌套 group 节点（回归测试：
/// build_struct_contents 重构曾丢失该传递，tag 错写到字节 0，破坏线格式）
#[test]
fn named_union_discriminant_offset_comes_from_nested_node() {
    let out = render_request(machine_request());
    let m = find(&out, "Machine.elm");
    assert!(
        m.contains("Capnproto.readUInt16 reader 6"),
        "getWhich must read the tag at discriminant_offset 3 (byte 6):\n{m}"
    );
    assert!(
        m.contains("Capnproto.writeUInt16 soffset 6 0 builder"),
        "encode must write branch tag 0 at byte 6:\n{m}"
    );
    assert!(
        m.contains("Capnproto.writeUInt16 soffset 6 1 builder"),
        "encode must write branch tag 1 at byte 6:\n{m}"
    );
}

/// interface schema：RPC 运行时 + interface 模块 + Server 子模块齐全
#[test]
fn interface_schema_renders_rpc_runtime_and_server() {
    let out = render_request(interface_request());
    find(&out, "Rpc/WebSocket.elm");
    find(&out, "Rpc/Client.elm");
    find(&out, "Ping.elm");
    find(&out, "Ping/Server.elm");
}

/// 回归锁（ca7c94f）：receive 原样透传（JS 侧已剥离长度前缀），
/// send 仍然 frameMessage —— 双重解帧曾把 capnp 段数词误读为帧长
#[test]
fn websocket_receive_is_raw_passthrough() {
    let out = render_request(interface_request());
    let ws = find(&out, "Rpc/WebSocket.elm");
    assert!(
        ws.contains("listToBytes data"),
        "receive must pass bytes through raw:\n{ws}"
    );
    assert!(
        !ws.contains("unframeMessage"),
        "unframeMessage is dead code (JS strips the prefix); module must not mention it:\n{ws}"
    );
    assert!(
        ws.contains("frameMessage"),
        "send path must still apply framing:\n{ws}"
    );
}

/// 含 interface 字段的 struct：导出 encodeWithCaps，capTable 按字段序生成
#[test]
fn interface_field_struct_exports_encode_with_caps() {
    let out = render_request(interface_request());
    let m = find(&out, "Holder.elm");
    assert!(
        m.contains("encodeWithCaps"),
        "struct with interface fields must export encodeWithCaps:\n{m}"
    );
    assert!(
        m.contains("Rpc.capSlot"),
        "encodeWithCaps must resolve caps via Rpc.capSlot:\n{m}"
    );
    assert!(
        m.contains("entity.cb1 :: entity.cb2 ::"),
        "capTable must list interface fields in declaration order:\n{m}"
    );
}
