# capnpc-elm

English | [中文](README-zh.md)

A Cap'n Proto schema to Elm code generator. Written in Rust, runs as a `capnp compile` plugin, reads `CodeGeneratorRequest` binary stream from stdin, and generates Elm encoder/decoder modules, RPC clients, and a WebSocket runtime.

## Features

- Auto-generates Elm encoders/decoders from `.capnp` schemas
- Supports struct, enum, union, group, and generic parameters
- Auto-generates RPC clients and WebSocket transport when the schema contains interfaces
- Generates server stub modules
- Output formatted with `elm-format` (optional; silently falls back to unformatted output if unavailable)

## Prerequisites

- [Rust](https://www.rust-lang.org/) (edition 2024)
- [capnp](https://capnproto.org/install.html) CLI tool (for schema compilation)
- [elm-format](https://github.com/avh4/elm-format) (optional, for beautifying generated Elm code)

## Build

```bash
cargo build --release
```

## Usage

The generator runs as a `capnp compile` plugin via the `-oelm` flag. Ensure the compiled binary is in your `PATH` (or specify the full path):

```bash
# Basic usage
capnp compile -oelm:<output-dir> schema.capnp

# With source prefix
capnp compile -oelm:src/elm schema.capnp --src-prefix=src/proto
```

`capnp compile` passes the `CodeGeneratorRequest` to the plugin via stdin. The plugin writes `.elm` files under `<output-dir>`.

### Generated Files

| File | Description |
|------|-------------|
| `<SchemaPath>/<Type>.elm` | Encoder/decoder module for each struct / enum in the schema |
| `<InterfacePath>/Server.elm` | Server module for each interface |
| `Capnproto.elm` | Fixed runtime module (Word64 type, etc.) |
| `Rpc/Client.elm` | RPC client runtime |
| `Rpc/WebSocket.elm` | WebSocket transport layer |

## Architecture

Code generation has three stages:

```
stdin (CodeGeneratorRequest)
  │
  ▼
parse_request()        ← binary message → ParsedSchema
  │
  ▼
bind_to_elm()          ← ParsedSchema → ElmContext (IR)
  │                       auto-loads rpc.capnp if interfaces exist
  ▼
render_elm()           ← ElmContext → .elm files (Askama templates)
```

### Source Structure

```
src/
  lib.rs           Public API: parse_request / bind_to_elm / render_elm / render_elm_to
  main.rs          CLI entry point
  capnproto.rs     Schema parsing: CodeGeneratorRequest → ParsedSchema
  elm.rs           Elm IR definitions: ElmType / ElmModule / ElmField / ElmContext, etc.
  binding.rs       Schema → Elm IR binding
  type_mapping.rs  Cap'n Proto → Elm type mapping
  render.rs        Askama template rendering + Elm-specific filters
  output.rs        Output strategies: FileWriter / MemoryWriter
  js/
    websocket.js   JS-side WebSocket port adapter (Elm ports interop)
templates/
  struct.j2        struct encoder/decoder template
  enum.j2          enum encoder/decoder template
  union.j2         union encoder/decoder template
  interface.j2     interface client template
  server.j2        interface server template
  rpc.j2           RPC client runtime
  websocket.j2     WebSocket transport
  runtime.j2       Fixed runtime (Capnproto.elm)
  module.j2        Common module header
```

## Wire Protocol

The generated RPC runtime speaks Cap'n Proto RPC over WebSocket binary frames with a length-prefix framing layer:

```
[ length: u32 little-endian ][ message bytes (length bytes) ]
```

- `message bytes` is one Cap'n Proto RPC message in the standard (unpacked) serialization format — the same bytes `capnp::serialize::write_message` produces. The packed encoding is not used.
- The length prefix counts payload bytes only (excluding the 4 prefix bytes).
- WebSocket frames are **not** a message boundary: one WS binary frame may batch several messages or carry a partial one. Receivers must reassemble strictly by the length prefix.
- Senders emit exactly one framed message per WS binary frame.

Three independent implementations depend on this framing; treat it as a compatibility contract:

| Implementation | Path |
|---|---|
| Browser JS port layer (Elm ports) | `src/js/websocket.js` |
| TypeScript test backend | `test-project/backend/src/services.ts` (`sendFrame` / `processBuffer`) |
| Rust WS↔stream bridge (for capnp-rpc peers) | `test-project/rust-interop/src/bridge.rs` |

## Testing

There are currently no Rust unit tests. `cargo test` compiles but does not execute any tests.

The recommended way to test generated output is to call `render_elm_to()` with `MemoryWriter` (`output.rs`), which collects rendered results into an in-memory HashMap without writing files or calling elm-format.

Integration verification is done in `test-project/` (gitignored): schema → code generation → Elm compilation → WebSocket RPC → E2E tests.

## Type Mapping

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
| `enum` | Custom Elm type |
| `struct` | Custom Elm record / type |
| `union` | `Union` type variant |
| `interface` | RPC client + server modules |

### Key Design Decisions

- **Int64 → `Capnproto.Word64`**: Elm's `Int` is arbitrary-precision, which doesn't match Cap'n Proto's 64-bit signed integer
- **Default value XOR mask**: Encoded values are XORed with defaults at encode time; zero-valued fields are skipped
- **Unions use `UnionInline`**: Both anonymous and named unions render as `Union` type variants
- **Group fields are flattened**: Group structs don't generate separate modules; their fields are flattened into the parent struct

## License

MIT
