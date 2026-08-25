# capnpc-elm

[English](README.md) | 中文

Cap'n Proto schema 到 Elm 的代码生成器。使用 Rust 编写，作为 `capnp compile` 的插件运行，从 stdin 接收 `CodeGeneratorRequest` 二进制流，生成 Elm 编解码模块、RPC 客户端和 WebSocket 运行时。

## 特性

- 从 `.capnp` schema 自动生成 Elm 类型的编解码器（encoder / decoder）
- 支持 struct、enum、union、group、泛型参数
- 自动生成 RPC 客户端和 WebSocket 传输层（当 schema 包含 interface 时）
- 生成服务端 stub 模块
- 输出经过 `elm-format` 格式化（可选；不可用时静默回退未格式化输出）

## 前置依赖

- [Rust](https://www.rust-lang.org/)（edition 2024）
- [capnp](https://capnproto.org/install.html) CLI 工具（用于 schema 编译）
- [elm-format](https://github.com/avh4/elm-format)（可选，用于美化生成的 Elm 代码）

## 构建

```bash
cargo build --release
```

## 使用

生成器作为 `capnp compile` 的 `-oelm` 插件运行。确保编译出的二进制在 `PATH` 中（或指定完整路径）：

```bash
# 基本用法
capnp compile -oelm:<output-dir> schema.capnp

# 带源路径前缀
capnp compile -oelm:src/elm schema.capnp --src-prefix=src/proto
```

`capnp compile` 会将 `CodeGeneratorRequest` 通过 stdin 传给插件，插件在 `<output-dir>` 下生成 `.elm` 文件。

### 生成的文件

| 文件 | 说明 |
|------|------|
| `<SchemaPath>/<Type>.elm` | schema 中每个 struct / enum 生成的编解码模块 |
| `<InterfacePath>/Server.elm` | 每个 interface 生成的服务端模块 |
| `Capnproto.elm` | 固定运行时模块（Word64 类型等） |
| `Rpc/Client.elm` | RPC 客户端运行时 |
| `Rpc/WebSocket.elm` | WebSocket 传输层 |

## 架构

代码生成分三个阶段：

```
stdin (CodeGeneratorRequest)
  │
  ▼
parse_request()        ← 二进制消息 → ParsedSchema
  │
  ▼
bind_to_elm()          ← ParsedSchema → ElmContext (IR)
  │                       若有 interface，自动加载 rpc.capnp
  ▼
render_elm()           ← ElmContext → .elm 文件（Askama 模板）
```

### 源码结构

```
src/
  lib.rs           公共 API：parse_request / bind_to_elm / render_elm / render_elm_to
  main.rs          CLI 入口
  capnproto.rs     Schema 解析：CodeGeneratorRequest → ParsedSchema
  elm.rs           Elm IR 定义：ElmType / ElmModule / ElmField / ElmContext 等
  binding.rs       Schema → Elm IR 绑定
  type_mapping.rs  Cap'n Proto → Elm 类型映射
  render.rs        Askama 模板渲染 + Elm 专用过滤器
  output.rs        输出策略：FileWriter / MemoryWriter
  js/
    websocket.js   JS 端 WebSocket 端口适配器（Elm ports 互操作）
templates/
  struct.j2        struct 编解码模板
  enum.j2          enum 编解码模板
  union.j2         union 编解码模板
  interface.j2     interface 客户端模板
  server.j2        interface 服务端模板
  rpc.j2           RPC 客户端运行时
  websocket.j2     WebSocket 传输
  runtime.j2       固定运行时（Capnproto.elm）
  module.j2        通用模块头
```

## 线路协议

生成的 RPC 运行时通过 WebSocket 二进制帧承载 Cap'n Proto RPC，外层为长度前缀分帧：

```
[ length: u32 小端 ][ 消息字节 (length 字节) ]
```

- 消息字节是一条 Cap'n Proto RPC 消息的标准（非 packed）序列化格式 —— 与 `capnp::serialize::write_message` 产出的字节相同，不使用 packed 编码。
- 长度前缀只计负载字节数（不含 4 字节前缀本身）。
- WebSocket 帧**不是**消息边界：一个 WS 二进制帧可能批量携带多条消息，也可能只含某条消息的一部分。接收方必须严格按长度前缀重组。
- 发送方每条消息恰好发一个 WS 二进制帧。

三个独立实现依赖此分帧格式，将其视为兼容性契约：

| 实现 | 路径 |
|---|---|
| 浏览器 JS 端口层（Elm ports） | `src/js/websocket.js` |
| TypeScript 测试后端 | `test-project/backend/src/services.ts`（`sendFrame` / `processBuffer`） |
| Rust WS↔流桥（对接 capnp-rpc） | `test-project/rust-interop/src/bridge.rs` |

## 测试

当前无 Rust 单元测试。`cargo test` 可编译但不会执行测试。

测试生成输出的推荐方式：调用 `render_elm_to()` + `MemoryWriter`（`output.rs`），收集渲染结果到内存 HashMap 而不写文件、不调 elm-format。

集成验证在 `test-project/`（gitignored）中进行：schema → 代码生成 → Elm 编译 → WebSocket RPC → E2E 测试。

## 类型映射

| Cap'n Proto | Elm |
|---|---|
| `Bool` | `Bool` |
| `Int8` / `Int16` / `Int32` | `Int` |
| `Int64` | `Capnproto.Word64` |
| `UInt8` / `UInt16` / `UInt32` / `UInt64` | `Int` |
| `Float32` / `Float64` | `Float` |
| `Text` | `String` |
| `Data` | `Bytes` (elm/bytes) |
| `List(T)` | `List T` |
| `enum` | 自定义 Elm 类型 |
| `struct` | 自定义 Elm record / type |
| `union` | `Union` 类型变体 |
| `interface` | RPC 客户端 + 服务端模块 |

### 关键设计决策

- **Int64 → `Capnproto.Word64`**：Elm 的 `Int` 是任意精度，不匹配 Cap'n Proto 的 64-bit 有符号整数
- **默认值 XOR mask**：编码时对默认值做 XOR，零值字段跳过
- **Union 用 `UnionInline`**：匿名/命名 union 都渲染为 `Union` 类型变体
- **Group 字段展平**：group struct 不生成独立模块，字段展平到父 struct

## 许可证

MIT
