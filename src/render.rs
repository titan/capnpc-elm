use crate::elm::{
    ElmContext, ElmDefaultValue, ElmEnumVariant, ElmField, ElmInterface, ElmMethod, ElmModule,
    ElmPrimitiveType, ElmType, ElmTypeDef, ElmUnionBranch,
};
use crate::output::{FileWriter, OutputWriter};
use askama::Template;
use heck::ToUpperCamelCase;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Template)]
#[template(path = "module.j2", escape = "none")]
struct ModuleTemplate<'a> {
    module: &'a ElmModule,
    full_module_name: &'a str,
    exports: Vec<String>,
    body: String,
}

#[derive(Template)]
#[template(path = "struct.j2", escape = "none")]
struct StructTemplate<'a> {
    module: &'a ElmModule,
    fields: &'a [ElmField],
    data_fields: &'a [&'a ElmField],
    pointer_fields: &'a [&'a ElmField],
    has_union: bool,
    union_field: Option<&'a ElmField>,
    generic_params: &'a [String],
    has_interface_fields: bool,
}

#[derive(Template)]
#[template(path = "enum.j2", escape = "none")]
struct EnumTemplate {
    variants: Vec<ElmEnumVariant>,
}

#[derive(Template)]
#[template(path = "interface.j2", escape = "none")]
struct InterfaceTemplate<'a> {
    interface_high: &'a str,
    interface_low: &'a str,
    methods: &'a [ElmMethod],
}

#[derive(Template)]
#[template(path = "runtime.j2", escape = "none")]
struct RuntimeTemplate;

#[derive(Template)]
#[template(path = "rpc.j2", escape = "none")]
struct RpcTemplate;

#[derive(Template)]
#[template(path = "websocket.j2", escape = "none")]
struct WebSocketTemplate;

// ── Server.elm 生成相关 ──────────────────────────────────

/// Server.elm 模板中每个方法的预计算数据
struct ServerMethodData {
    id: u16,
    name: String,
    param_module: String,
    result_module: String,
    param_has_caps: bool,
}

#[derive(Template)]
#[template(path = "server.j2", escape = "none")]
struct ServerTemplate {
    full_module_name: String,
    interface_high: String,
    interface_low: String,
    methods: Vec<ServerMethodData>,
    imports: Vec<String>,
}

// 渲染所有模块到自定义 OutputWriter
pub fn render_elm_modules_to(
    context: &ElmContext,
    writer: &dyn OutputWriter,
) -> anyhow::Result<()> {
    let mut outputs: Vec<(PathBuf, String)> = Vec::new();

    // 渲染运行时模块
    outputs.push(render_runtime_module()?);

    let found_rpc = context.has_interfaces();
    // Render all type modules
    for module in &context.modules {
        outputs.push(render_module(module)?);
    }

    if found_rpc {
        outputs.push(render_rpc_module()?);
        outputs.push(render_websocket_module()?);

        // 为每个 interface 生成 Server.elm 子模块
        for module in &context.modules {
            if matches!(module.type_def, ElmTypeDef::Interface) {
                outputs.push(render_server_module(module)?);
            }
        }
    }

    for (path, content) in outputs {
        writer.write(&path, &content)?;
    }

    Ok(())
}

// 渲染所有模块（使用 FileWriter）
pub fn render_elm_modules(context: &ElmContext) -> anyhow::Result<()> {
    render_elm_modules_to(context, &FileWriter)
}

// 渲染运行时模块
fn render_runtime_module() -> anyhow::Result<(PathBuf, String)> {
    let runtime = RuntimeTemplate {};
    let content = runtime.render().expect("Failed to render runtime module");
    Ok((PathBuf::from("Capnproto.elm"), content))
}

// 渲染 RPC 模块
fn render_rpc_module() -> anyhow::Result<(PathBuf, String)> {
    let rpc = RpcTemplate {};
    let content = rpc.render().expect("Failed to render runtime module");
    Ok((PathBuf::from("Rpc/Client.elm"), content))
}

// Render WebSocket module
fn render_websocket_module() -> anyhow::Result<(PathBuf, String)> {
    let websocket = WebSocketTemplate {};
    let content = websocket
        .render()
        .expect("Failed to render WebSocket module");
    Ok((PathBuf::from("Rpc/WebSocket.elm"), content))
}

// 渲染 interface 的 Server.elm 子模块
fn render_server_module(module: &ElmModule) -> anyhow::Result<(PathBuf, String)> {
    let full_module_name = if module.path.is_empty() {
        module.name.clone()
    } else {
        format!("{}.{}", module.path, module.name)
    };

    let high = format!("0x{:08X}", (module.id >> 32) as u32);
    let low = format!("0x{:08X}", (module.id & 0xFFFFFFFF) as u32);

    // 预计算每个方法的参数/结果模块路径
    let methods: Vec<ServerMethodData> = module
        .methods
        .iter()
        .map(|m| ServerMethodData {
            id: m.id,
            name: m.name.clone(),
            param_module: m.param_type.module_name(),
            result_module: m.result_type.module_name(),
            param_has_caps: m.param_has_caps,
        })
        .collect();

    // 去重收集所有需要 import 的模块
    let mut imports: Vec<String> = Vec::new();
    for m in &methods {
        if !m.param_module.is_empty() && !imports.contains(&m.param_module) {
            imports.push(m.param_module.clone());
        }
        if !m.result_module.is_empty() && !imports.contains(&m.result_module) {
            imports.push(m.result_module.clone());
        }
    }

    let template = ServerTemplate {
        full_module_name: full_module_name.clone(),
        interface_high: high,
        interface_low: low,
        methods,
        imports,
    };

    let content = template
        .render()
        .expect("Failed to render server module");

    // 文件路径: FullModuleName/Server.elm
    let file_path = full_module_name
        .split('.')
        .map(|s| s.to_upper_camel_case())
        .collect::<Vec<_>>()
        .join("/");
    let file_name = format!("{}/Server.elm", file_path);

    Ok((PathBuf::from(&file_name), content))
}

/// 合并成员的符号前缀表：全部导出符号 + 模块级内部符号（Union/dataWords/…）
fn member_symbol_prefix(member_name: &str, symbols: &[String]) -> HashMap<String, String> {
    let lower: String = {
        let mut c = member_name.chars();
        match c.next() {
            Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }
    };
    let cap = |s: &str| -> String {
        let mut c = s.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }
    };
    let mut map = HashMap::new();
    let mut push = |sym: &str| {
        let target = if sym.starts_with(|c: char| c.is_ascii_uppercase()) {
            format!("{member_name}{sym}")
        } else {
            format!("{lower}{}", cap(sym))
        };
        map.entry(sym.to_owned()).or_insert(target);
    };
    for s in symbols {
        let s = s.split('(').next().unwrap_or(s); // "Union(..)" → "Union"
        push(s);
    }
    for s in ["Union", "dataWords", "pointerWords", "getWhich", "layout"] {
        push(s);
    }
    map
}

/// 标识符级整词替换（逐字符扫描）。
/// 前面紧跟 `.` 的标识符是模块限定引用（如 Semantica.JsonValue.Entity），
/// 必须保持原名 —— 只有裸标识符（本成员的顶层符号）才加前缀。
fn rename_identifiers(text: &str, map: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len() + 32);
    let mut word = String::new();
    let mut last_significant: Option<char> = None;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            word.push(ch);
        } else {
            if !word.is_empty() {
                let qualified = last_significant == Some('.');
                if qualified {
                    out.push_str(&word);
                } else if let Some(rep) = map.get(word.as_str()) {
                    out.push_str(rep);
                } else {
                    out.push_str(&word);
                }
                word.clear();
            }
            // 空白打断限定引用（Elm 的 `Mod.ident` 中点与标识符之间无空白）
            last_significant = if ch.is_whitespace() { None } else { Some(ch) };
            out.push(ch);
        }
    }
    if !word.is_empty() {
        let qualified = last_significant == Some('.');
        if qualified {
            out.push_str(&word);
        } else if let Some(rep) = map.get(word.as_str()) {
            out.push_str(rep);
        } else {
            out.push_str(&word);
        }
    }
    out
}

/// 剥离 `canon_prefix + 顶层符号` 的自限定引用（Semantica.JsonValue.Entity → Entity）。
/// 子模块引用（Semantica.JsonValue.Null.Entity）不是顶层符号，保留完整路径——
/// Elm 的限定名是完整导入路径，不能只写最后一段。
fn strip_canon_prefix(body: &str, canon_prefix: &str, symbols: &[String]) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(pos) = rest.find(canon_prefix) {
        let after = &rest[pos + canon_prefix.len()..];
        let ident_end = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(after.len());
        let ident = &after[..ident_end];
        out.push_str(&rest[..pos]);
        if symbols.iter().any(|s| s == ident) {
            out.push_str(ident);
        } else {
            out.push_str(&rest[pos..pos + canon_prefix.len() + ident_end]);
        }
        rest = &rest[pos + canon_prefix.len() + ident_end..];
    }
    out.push_str(rest);
    out
}

/// 渲染 SCC 合并模块：canonical 分片 + 次成员分片（符号加前缀）拼接。
fn render_merged_module(canonical: &ElmModule) -> anyhow::Result<(PathBuf, String)> {
    let full_module_name = format!("{}.{}", canonical.path, canonical.name);

    let mut bodies: Vec<String> = vec![];
    let mut exports: Vec<String> = vec![];

    // canonical 分片：原名原符号（对自身模块前缀同样剥离——模块内不能自限定）
    let hf_canon = canonical
        .fields
        .iter()
        .any(|f| f.elm_type.contains_interface_ref());
    exports.extend(generate_exports(canonical, hf_canon));
    let canon_prefix = format!("{}.", full_module_name);
    let mut canon_symbols: Vec<String> = exports.clone();
    for extra in ["Union", "dataWords", "pointerWords", "getWhich", "layout"] {
        canon_symbols.push(extra.to_owned());
    }
    let canon_body = render_struct(canonical, hf_canon)?;

    // 次成员分片：全部导出符号加成员类型名前缀
    for member in &canonical.merged_members {
        let hf = member
            .fields
            .iter()
            .any(|f| f.elm_type.contains_interface_ref());
        let symbols = generate_exports(member, hf);
        let body = render_struct(member, hf)?;
        let map = member_symbol_prefix(&member.name, &symbols);
        let mut body = rename_identifiers(&body, &map);
        // layout 记录的字段名是 Capnproto.StructLayout 的固定接口，不随符号前缀改：
        // `propDataWords = propDataWords` → `dataWords = propDataWords`
        body = body
            .replace("propDataWords = propDataWords", "dataWords = propDataWords")
            .replace(
                "propPointerWords = propPointerWords",
                "pointerWords = propPointerWords",
            );
        let body = strip_canon_prefix(&body, &canon_prefix, &canon_symbols);
        bodies.push(body);
        for sym in symbols {
            let sym_head = sym.split('(').next().unwrap_or(&sym).to_string();
            if let Some(renamed) = map.get(&sym_head) {
                let renamed = renamed.clone();
                canon_symbols.push(renamed.clone());
                if sym.contains('(') {
                    exports.push(format!("{renamed}(..)"));
                } else {
                    exports.push(renamed);
                }
            }
        }
    }

    // canonical 剥离放到成员符号入白名单之后（其分片会引用 PropEntity 等）
    bodies.insert(
        0,
        strip_canon_prefix(&canon_body, &canon_prefix, &canon_symbols),
    );

    let template = ModuleTemplate {
        module: canonical,
        full_module_name: if canonical.path.is_empty() {
            &canonical.name
        } else {
            &full_module_name
        },
        exports,
        body: bodies.join("\n"),
    };
    let content = template.render().expect("Failed to render module");

    let file_path = full_module_name
        .split('.')
        .map(|seg| seg.to_upper_camel_case())
        .collect::<Vec<_>>()
        .join("/");
    Ok((PathBuf::from(format!("{file_path}.elm")), content))
}

// 渲染单个模块
fn render_module(module: &ElmModule) -> anyhow::Result<(PathBuf, String)> {
    if !module.merged_members.is_empty() {
        return render_merged_module(module);
    }
    // 准备模块数据
    let full_module_name = format!("{}.{}", module.path, module.name);
    let has_interface_fields = module
        .fields
        .iter()
        .any(|f| f.elm_type.contains_interface_ref());
    let exports = generate_exports(module, has_interface_fields);

    let body = match module.type_def {
        ElmTypeDef::Struct => render_struct(module, has_interface_fields).unwrap_or_else(|e| {
            eprintln!("Failed to render struct: {}", e);
            String::new()
        }),
        ElmTypeDef::Enum => render_enum(module).unwrap_or_else(|e| {
            eprintln!("Failed to render enum: {}", e);
            String::new()
        }),
        ElmTypeDef::Interface => render_interface(module).unwrap_or_else(|e| {
            eprintln!("Failed to render interface: {}", e);
            String::new()
        }),
    };

    // 创建模板
    let template = ModuleTemplate {
        module,
        full_module_name: if module.path.is_empty() {
            &module.name
        } else {
            &format!("{}.{}", module.path, module.name)
        },
        exports,
        body,
    };

    // 渲染内容
    let content = template.render().expect("Failed to render module");

    // 创建文件路径：将模块名中的每个段转换为驼峰命名法
    let file_path = full_module_name
        .split('.')
        .map(|s| s.to_upper_camel_case())
        .collect::<Vec<_>>()
        .join("/");
    let file_name = format!("{}.elm", file_path);

    Ok((PathBuf::from(&file_name), content))
}

// 渲染单个接口
fn render_interface(module: &ElmModule) -> askama::Result<String> {
    // 转换 ElmModule 为 ElmInterface
    let interface = ElmInterface {
        id: module.id, // 需要确保 ElmModule 有 id
        name: module.name.clone(),
        path: module.path.clone(),
        methods: module.methods.clone(), // 需要确保 ElmModule 有 methods
    };

    let high = format!("0x{:08X}", (interface.id >> 32) as u32);
    let low = format!("0x{:08X}", (interface.id & 0xFFFFFFFF) as u32);

    let template = InterfaceTemplate {
        interface_high: &high,
        interface_low: &low,
        methods: &interface.methods,
    };

    template.render()
}

// 提取 Enum Variant
fn extract_enum_variants(fields: &[ElmField]) -> Vec<ElmEnumVariant> {
    fields
        .iter()
        .filter_map(|field| {
            if let ElmType::EnumRef(_, _, ref variants, _) = field.elm_type {
                Some(variants.clone())
            } else {
                None
            }
        })
        .flatten()
        .collect()
}

// 生成导出列表
fn generate_exports(module: &ElmModule, has_interface_fields: bool) -> Vec<String> {
    let mut exports = vec![];

    match module.type_def {
        ElmTypeDef::Struct => {
            exports.push("Entity".to_string());
            exports.push("dataWords".to_string());
            exports.push("pointerWords".to_string());
            exports.push("encode".to_string());
            if has_interface_fields {
                exports.push("encodeWithCaps".to_string());
            }
            exports.push("decode".to_string());
            exports.push("toAnyPointer".to_string());
            exports.push("fromAnyPointer".to_string());

            // 添加字段访问器
            for field in &module.fields {
                // 对于列表字段，添加计数和索引访问器
                if field.elm_type.is_list() {
                    exports.push(format!("get{}Count", field.name.to_upper_camel_case()));
                    exports.push(format!("get{}At", field.name.to_upper_camel_case()));
                } else if field.elm_type.is_anypointer() {
                    exports.push(format!("get{}Reader", field.name.to_upper_camel_case()));
                    exports.push(format!("get{}", field.name.to_upper_camel_case()));
                } else if field.elm_type.is_struct_ref() {
                    exports.push(format!("get{}Reader", field.name.to_upper_camel_case()));
                    exports.push(format!("get{}", field.name.to_upper_camel_case()));
                } else if let ElmType::UnionInline(branches, _) = &field.elm_type {
                    exports.push("Union(..)".to_owned());
                    for branch in branches {
                        if !branch.elm_type.is_void() {
                            exports.push(format!("get{}", branch.name.to_upper_camel_case()));
                            if branch.elm_type.is_struct_ref()
                                || branch.elm_type.is_anypointer()
                                || branch.elm_type.is_generic_param()
                            {
                                exports.push(format!(
                                    "get{}Reader",
                                    branch.name.to_upper_camel_case()
                                ));
                            }
                        }
                        exports.push(format!("is{}", branch.name.to_upper_camel_case()));
                    }
                } else if !field.elm_type.is_void() && field.elm_type.to_elm_string() != "Union" {
                    exports.push(format!("get{}", field.name.to_upper_camel_case()));
                }
            }
        }
        ElmTypeDef::Enum => {
            exports.push("Entity(..)".to_string());
            exports.push("fromCode".to_string());
            exports.push("toCode".to_string());
        }
        ElmTypeDef::Interface => {
            exports.push("interfaceId".to_string());
            for method in &module.methods {
                exports.push(method.name.to_owned());
                for param in &method.implicit_parameters {
                    exports.push(param.clone());
                }
            }
        }
    }

    exports
}

fn render_struct(module: &ElmModule, has_interface_fields: bool) -> askama::Result<String> {
    let mut data_fields = Vec::new();
    let mut pointer_fields = Vec::new();
    let mut has_union = false;
    let mut union_field = None;

    // 分离数值字段和指针字段
    for field in &module.fields {
        match &field.elm_type {
            ElmType::Primitive(ElmPrimitiveType::Unit) => (),
            ElmType::Primitive(ElmPrimitiveType::Bool) => {
                data_fields.push(field);
            }
            ElmType::Primitive(ElmPrimitiveType::String) => {
                pointer_fields.push(field);
            }
            ElmType::Primitive(ElmPrimitiveType::Bytes) => {
                pointer_fields.push(field);
            }
            ElmType::Primitive(_) => {
                data_fields.push(field);
            }
            ElmType::StructRef(_, _, _) => {
                pointer_fields.push(field);
            }
            ElmType::EnumRef(_, _, _, _) => {
                data_fields.push(field);
            }
            ElmType::InterfaceRef(_, _, _) => {
                pointer_fields.push(field);
            }
            ElmType::AnyPointer => {
                pointer_fields.push(field);
            }
            ElmType::List(_) => pointer_fields.push(field),
            ElmType::UnionInline(branches, _) => {
                has_union = true;
                union_field = Some(field);
                // 处理联合的分支字段
                let mut found_data = false;
                let mut found_pointer = false;
                for branch in branches {
                    if branch.is_pointer {
                        found_pointer = true;
                    } else if branch.elm_type.is_void() {
                    } else {
                        found_data = true;
                    }
                }
                if found_pointer {
                    pointer_fields.push(field);
                }
                if found_data {
                    data_fields.push(field);
                }
            }
            ElmType::GenericParam(_) => {
                pointer_fields.push(field);
            }
        }
    }

    // 创建结构体模板
    let struct_template = StructTemplate {
        module,
        fields: &module.fields,
        data_fields: &data_fields,
        pointer_fields: &pointer_fields,
        has_union,
        union_field,
        generic_params: &module.generic_params,
        has_interface_fields,
    };

    struct_template.render()
}

fn render_enum(module: &ElmModule) -> askama::Result<String> {
    let variants = extract_enum_variants(&module.fields);

    let enum_template = EnumTemplate { variants };

    enum_template.render()
}

// Askama 过滤器函数
mod filters {
    use super::*;
    use lazy_static::lazy_static;
    use std::collections::HashSet;

    // Elm 语言的关键字列表
    lazy_static! {
        static ref ELM_KEYWORDS: HashSet<&'static str> = {
            let mut set = HashSet::new();
            set.insert("type");
            set.insert("module");
            set.insert("exposing");
            set.insert("import");
            set.insert("as");
            set.insert("if");
            set.insert("then");
            set.insert("else");
            set.insert("case");
            set.insert("of");
            set.insert("let");
            set.insert("in");
            set.insert("infix");
            set.insert("infixl");
            set.insert("infixr");
            set.insert("port");
            set.insert("alias");
            set.insert("where");
            set.insert("true");
            set.insert("false");
            set
        };
    }

    // ── 类型字符串过滤器 ────────────────────────────────

    pub fn elm_type_str(elm_type: &ElmType) -> askama::Result<String> {
        Ok(elm_type.to_elm_string())
    }

    pub fn elm_type_module_str(elm_type: &ElmType) -> askama::Result<String> {
        Ok(elm_type.module_name())
    }

    pub fn parenthesize_if_needed(elm_type: &ElmType) -> askama::Result<String> {
        let s = elm_type.to_elm_string();
        if s.contains(' ') {
            Ok(format!("({})", s))
        } else {
            Ok(s)
        }
    }

    // ── 读写类型/字节宽度过滤器 ──────────────────────────

    pub fn elm_write_type(elm_type: &ElmType) -> askama::Result<String> {
        Ok(elm_type.write_type())
    }

    pub fn elm_read_type(elm_type: &ElmType) -> askama::Result<String> {
        Ok(elm_type.read_type())
    }

    pub fn elm_type_bytewidth(elm_type: &ElmType) -> askama::Result<u32> {
        Ok(elm_type.byte_width())
    }

    // ── 类型谓词过滤器（薄包装） ────────────────────────

    pub fn is_boolean_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(elm_type.is_boolean())
    }

    pub fn is_integer_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(elm_type.is_integer())
    }

    pub fn is_float_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(elm_type.is_float())
    }

    pub fn is_text_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(elm_type.is_text())
    }

    pub fn is_data_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(elm_type.is_data())
    }

    pub fn is_struct_ref(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(elm_type.is_struct_ref())
    }

    pub fn is_interface_ref(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(elm_type.is_interface_ref())
    }

    pub fn is_enum_ref(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(elm_type.is_enum_ref())
    }

    pub fn is_list_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(elm_type.is_list())
    }

    pub fn is_union_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(elm_type.is_union())
    }

    pub fn is_anypointer_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(elm_type.is_anypointer())
    }

    pub fn is_void_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(elm_type.is_void())
    }

    pub fn is_generic_param_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(elm_type.is_generic_param())
    }

    // ── 默认值 XOR mask 过滤器 ──────────────────────────

    /// Returns true if the field has a non-zero default value requiring XOR mask
    pub fn has_nonzero_default(field: &ElmField) -> askama::Result<bool> {
        Ok(field
            .default_value
            .as_ref()
            .map_or(false, |dv| !dv.is_zero()))
    }

    /// Returns true if the field has a boolean default of true (requires bit inversion)
    pub fn has_bool_default_true(field: &ElmField) -> askama::Result<bool> {
        Ok(matches!(
            &field.default_value,
            Some(ElmDefaultValue::Bool(true))
        ))
    }

    /// Returns true if the field has a float default value (requires byte-level XOR)
    pub fn has_float_default(field: &ElmField) -> askama::Result<bool> {
        Ok(matches!(
            &field.default_value,
            Some(ElmDefaultValue::Float(_))
        ))
    }

    /// Returns the Elm literal for the default value (for use in Bitwise.xor)
    pub fn default_xor_mask(field: &ElmField) -> askama::Result<String> {
        Ok(field
            .default_value
            .as_ref()
            .map_or("0".to_string(), |dv| dv.to_elm_literal()))
    }

    /// Returns true if a union branch has a non-zero default value
    pub fn branch_has_nonzero_default(branch: &ElmUnionBranch) -> askama::Result<bool> {
        Ok(branch
            .default_value
            .as_ref()
            .map_or(false, |dv| !dv.is_zero()))
    }

    /// Returns true if a union branch has a boolean default of true
    pub fn branch_has_bool_default_true(branch: &ElmUnionBranch) -> askama::Result<bool> {
        Ok(matches!(
            &branch.default_value,
            Some(ElmDefaultValue::Bool(true))
        ))
    }

    /// Returns true if a union branch has a float default value
    pub fn branch_has_float_default(branch: &ElmUnionBranch) -> askama::Result<bool> {
        Ok(matches!(
            &branch.default_value,
            Some(ElmDefaultValue::Float(_))
        ))
    }

    /// Returns the Elm literal for the branch's default value
    pub fn branch_default_xor_mask(branch: &ElmUnionBranch) -> askama::Result<String> {
        Ok(branch
            .default_value
            .as_ref()
            .map_or("0".to_string(), |dv| dv.to_elm_literal()))
    }

    // ── 类型解构过滤器 ──────────────────────────────────

    pub fn strip_list_type(elm_type: &ElmType) -> askama::Result<ElmType> {
        let result = match elm_type {
            ElmType::List(inner) => *inner.clone(),
            tipe => tipe.clone(),
        };
        Ok(result)
    }

    pub fn strip_union_branches(elm_type: &ElmType) -> askama::Result<Vec<ElmUnionBranch>> {
        match elm_type {
            ElmType::UnionInline(branches, _) => Ok(branches.to_owned()),
            _ => Ok(vec![]),
        }
    }

    pub fn strip_type_args(elm_type: &ElmType) -> askama::Result<Vec<ElmType>> {
        match elm_type {
            ElmType::StructRef(_, _, args) => Ok(args.clone()),
            ElmType::EnumRef(_, _, _, args) => Ok(args.clone()),
            ElmType::InterfaceRef(_, _, args) => Ok(args.clone()),
            _ => Ok(vec![]),
        }
    }

    // ── 编解码过滤器（委托 ElmType 方法） ────────────────

    pub fn type_to_encoder(elm_type: &ElmType) -> askama::Result<String> {
        Ok(elm_type.encoder_expr())
    }

    pub fn type_to_decoder(elm_type: &ElmType) -> askama::Result<String> {
        Ok(elm_type.decoder_expr())
    }

    /// 跨模块 decode 函数名（合并次成员 → propDecode）
    pub fn qualified_decode(elm_type: &ElmType) -> askama::Result<String> {
        Ok(elm_type.qualified_decode_fn())
    }

    /// 跨模块 encode 函数名（合并次成员 → propEncode）
    pub fn qualified_encode(elm_type: &ElmType) -> askama::Result<String> {
        Ok(elm_type.qualified_encode_fn())
    }

    /// 跨模块 dataWords 常量名（合并次成员 → propDataWords）
    pub fn qualified_data_words(elm_type: &ElmType) -> askama::Result<String> {
        Ok(elm_type.qualified_data_words())
    }

    /// 跨模块 pointerWords 常量名（合并次成员 → propPointerWords）
    pub fn qualified_pointer_words(elm_type: &ElmType) -> askama::Result<String> {
        Ok(elm_type.qualified_pointer_words())
    }

    // ── 联合分支计数过滤器 ──────────────────────────────

    pub fn pointer_branches_count(branches: &[ElmUnionBranch]) -> askama::Result<usize> {
        let mut count = 0;
        for branch in branches {
            if branch.is_pointer {
                count += 1;
            }
        }
        Ok(count)
    }

    pub fn data_branches_count(branches: &[ElmUnionBranch]) -> askama::Result<usize> {
        let mut count = 0;
        for branch in branches {
            if branch.is_pointer || branch.elm_type.is_void() {
                continue;
            }

            count += 1;
        }
        Ok(count)
    }

    pub fn branches_count(branches: &[ElmUnionBranch]) -> askama::Result<usize> {
        Ok(branches.len())
    }

    // ── 命名工具过滤器 ──────────────────────────────────

    pub fn to_upper_camel_case(s: &str) -> askama::Result<String> {
        Ok(s.to_upper_camel_case())
    }

    /// Union 分支名 → Elm 构造器名，避开 Prelude 构造器（`ok` → `Ok` 会遮蔽
    /// `Result.Ok`，令模板里 decode 的 `Ok Entity` 包装解析错误）。
    pub fn to_union_ctor(s: &str) -> askama::Result<String> {
        let ctor = s.to_upper_camel_case();
        Ok(match ctor.as_str() {
            "Ok" | "Err" | "Just" | "Nothing" | "LT" | "EQ" | "GT" => format!("{ctor}Value"),
            _ => ctor,
        })
    }

    pub fn escape_elm_keyword(s: &str) -> askama::Result<String> {
        // 检查字符串是否是 Elm 关键字
        if ELM_KEYWORDS.contains(s) {
            // 如果是关键字，在末尾添加下划线
            Ok(format!("{}_", s))
        } else {
            // 否则返回原始字符串
            Ok(s.to_string())
        }
    }

    // ── 列表编解码过滤器（委托 ElmType 方法） ────────────

    /// Returns the full list encoder expression for use in templates
    pub fn list_encoder_expr(elm_type: &ElmType) -> askama::Result<String> {
        Ok(elm_type.list_encoder_expr())
    }

    /// Returns the list element decoder expression for getXAt functions
    pub fn list_element_reader_expr(elm_type: &ElmType) -> askama::Result<String> {
        Ok(elm_type.list_element_reader_expr())
    }
}
