use crate::elm::{
    ElmContext, ElmDefaultValue, ElmEnumVariant, ElmField, ElmInterface, ElmMethod, ElmModule,
    ElmPrimitiveType, ElmType, ElmTypeDef, ElmUnionBranch,
};
use crate::output::{FileWriter, OutputWriter};
use askama::Template;
use heck::ToUpperCamelCase;
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
struct EnumTemplate<'a> {
    module: &'a ElmModule,
    variants: Vec<ElmEnumVariant>,
}

#[derive(Template)]
#[template(path = "interface.j2", escape = "none")]
struct InterfaceTemplate<'a> {
    module: &'a ElmInterface,
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

// 渲染单个模块
fn render_module(module: &ElmModule) -> anyhow::Result<(PathBuf, String)> {
    // 准备模块数据
    let full_module_name = format!("{}.{}", module.path, module.name);
    let exports = generate_exports(module);

    let body = match module.type_def {
        ElmTypeDef::Struct => render_struct(module).unwrap_or_else(|e| {
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
        module: &interface,
        interface_high: &high,
        interface_low: &low,
        methods: &interface.methods,
    };

    template.render()
}

// 提取联合分支
fn extract_union_branches(fields: &[ElmField]) -> Vec<ElmUnionBranch> {
    fields
        .iter()
        .filter_map(|field| {
            if let ElmType::UnionInline(ref branches, _) = field.elm_type {
                Some(branches.clone())
            } else {
                None
            }
        })
        .flatten()
        .collect()
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
fn generate_exports(module: &ElmModule) -> Vec<String> {
    let mut exports = vec![];

    match module.type_def {
        ElmTypeDef::Struct => {
            exports.push("Entity".to_string());
            exports.push("dataWords".to_string());
            exports.push("pointerWords".to_string());
            eprintln!(
                "DEBUG exports for {}: dataWords, pointerWords added, total={}",
                module.name,
                exports.len()
            );
            exports.push("encode".to_string());
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
                } else if !field.elm_type.is_void()
                    && field.elm_type.to_elm_string() != "Union"
                {
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

fn render_struct(module: &ElmModule) -> askama::Result<String> {
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

    let has_interface_fields = module
        .fields
        .iter()
        .any(|f| f.elm_type.contains_interface_ref());

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

    let enum_template = EnumTemplate { module, variants };

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

    pub fn is_primitive_list(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(elm_type.is_primitive_list())
    }

    pub fn is_union_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(elm_type.is_union())
    }

    pub fn is_anypointer_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(elm_type.is_anypointer())
    }

    pub fn is_pointer_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(elm_type.is_pointer())
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

    /// Returns the Capnproto list encoder function name for a list type
    pub fn list_encoder_name(elm_type: &ElmType) -> askama::Result<String> {
        match elm_type {
            ElmType::List(inner) => match inner.as_ref() {
                ElmType::Primitive(ElmPrimitiveType::Int(_)) => {
                    Ok("Capnproto.encodePrimitiveIntList".to_string())
                }
                ElmType::Primitive(ElmPrimitiveType::Bool) => {
                    Ok("Capnproto.encodeBoolList".to_string())
                }
                ElmType::Primitive(ElmPrimitiveType::Float(_)) => {
                    Ok("Capnproto.encodePrimitiveList".to_string())
                }
                ElmType::Primitive(ElmPrimitiveType::String) => {
                    Ok("Capnproto.encodeTextList".to_string())
                }
                ElmType::StructRef(_, _, _) => Ok("Capnproto.encodeStructList".to_string()),
                _ => Ok("Capnproto.encodeStructList".to_string()),
            },
            _ => Ok("Capnproto.encodeStructList".to_string()),
        }
    }

    /// Returns true if the list element type needs a struct-style encoder
    pub fn is_struct_list(elm_type: &ElmType) -> askama::Result<bool> {
        match elm_type {
            ElmType::List(inner) => Ok(matches!(inner.as_ref(), ElmType::StructRef(_, _, _))),
            _ => Ok(false),
        }
    }

    /// Returns true if the list element type is a primitive (non-struct)
    pub fn is_primitive_list_type(elm_type: &ElmType) -> askama::Result<bool> {
        match elm_type {
            ElmType::List(inner) => Ok(matches!(inner.as_ref(), ElmType::Primitive(_))),
            _ => Ok(false),
        }
    }

    /// Returns the full list encoder expression for use in templates
    pub fn list_encoder_expr(elm_type: &ElmType) -> askama::Result<String> {
        Ok(elm_type.list_encoder_expr())
    }

    /// Returns the list element decoder expression for getXAt functions
    pub fn list_element_reader_expr(elm_type: &ElmType) -> askama::Result<String> {
        Ok(elm_type.list_element_reader_expr())
    }
}
