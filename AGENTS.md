# AGENTS.md — capnpc-elm3

Cap'n Proto → Elm 代码生成器（Rust）。读取 `capnp compile` 的 `CodeGeneratorRequest` 二进制流，生成 Elm 编解码模块 + RPC 客户端 + WebSocket 运行时。

## 构建 & 运行

```bash
cargo build                    # 编译生成器二进制
cargo run                      # 从 stdin 读 CodeGeneratorRequest → 输出到当前目录
# 实际使用方式：
capnp compile -oelm:<output-dir> <schema.capnp> --src-prefix=<prefix>
```

生成器作为 `capnp compile -oelm` 插件运行，从 stdin 接收 `CodeGeneratorRequest`。

## 测试

单测在 `src/tests.rs`（7 个，经 `render_elm_to()` + `MemoryWriter` 走 parse/bind/render 管线，不写文件、不调 elm-format）。`./verify.sh` = cargo test + 全量 E2E（test-project 在盘时）；`./verify.sh --fast` 只跑单测。

集成验证在 `test-project/`（gitignored）中进行：schema → 代码生成 → Elm 编译 → WebSocket RPC → E2E 测试。

## 三阶段管线

1. **`parse_request()`** (`lib.rs`) — 从 `capnp::serialize` 读取二进制消息 → `ParsedSchema`
2. **`bind_to_elm()`** (`binding.rs`) — 将 schema nodes 转为 `ElmContext`（IR）；若有 interface，自动加载系统 `rpc.capnp`
3. **`render_elm()`** (`render.rs`) — Askama 模板渲染 → `.elm` 文件，写入时调用 `elm-format`

## 源码结构

| 文件 | 职责 |
|------|------|
| `src/lib.rs` | 公共 API：`parse_request` / `bind_to_elm` / `render_elm` / `render_elm_to` |
| `src/main.rs` | CLI 入口：stdin → `generate_elm_code()` |
| `src/capnproto.rs` | Schema 解析：`code_generator_request` → `ParsedSchema` |
| `src/elm.rs` | Elm IR 定义：`ElmType` / `ElmModule` / `ElmField` / `ElmContext` 等 |
| `src/binding.rs` | Schema → Elm IR 绑定逻辑 |
| `src/type_mapping.rs` | Cap'n Proto → Elm 类型映射（含缓存、import 收集） |
| `src/render.rs` | Askama 模板渲染 + Elm 专用过滤器 |
| `src/output.rs` | 输出策略：`FileWriter`（+elm-format）/ `MemoryWriter`（测试用） |
| `templates/*.j2` | Askama Jinja2 模板（struct / enum / interface / rpc / runtime / websocket / server / module） |
| `src/js/websocket.js` | JS 端 WebSocket 端口适配器（Elm ports 互操作） |

## 外部依赖

- **运行时依赖**: `capnp` CLI 工具（提供 schema 编译），`elm-format`（可选，美化输出）
- **rpc.capnp 自动加载**: 当 schema 含 interface 时，自动查找 `/usr/include/capnp/rpc.capnp` 或 `/usr/local/include/capnp/rpc.capnp`
- **elm-format 容错**: 若 `elm-format` 不可用或执行失败，静默回退输出未格式化代码（不报错）

## 关键设计决策

- **Int64 → `Capnproto.Word64`**: Elm 的 `Int` 是任意精度，不匹配 Cap'n Proto 的 64-bit 有符号整数，因此生成自定义 `Word64` 类型
- **UInt 全部映射为 Elm `Int`**: Elm 没有 unsigned int 概念
- **默认值用 XOR mask**: Cap'n Proto wire format 要求编码时 XOR 默认值；`is_zero()` 为 true 时跳过
- **Union 用 `UnionInline`**: 匿名/命名 union 都渲染为 `Union` 类型变体
- **Group 字段跳过**: group struct 不生成独立模块，其字段展平到父 struct

## 工具链注意

- Rust edition **2024**（`Cargo.toml` 中指定）
- Askama 模板位于项目根 `templates/`（非 `src/templates/`），这是 askama 默认搜索路径
- 无 CI 配置、无 lint/rustfmt/clippy 配置文件
