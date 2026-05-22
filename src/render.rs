use crate::elm::{
    ElmContext, ElmDefaultValue, ElmEnumVariant, ElmField, ElmInterface, ElmMethod, ElmModule,
    ElmPrimitiveType, ElmType, ElmTypeDef, ElmUnionBranch,
};
use askama::Template;
use heck::ToUpperCamelCase;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

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

// 渲染所有模块
pub fn render_elm_modules(context: &ElmContext) -> anyhow::Result<()> {
    // 渲染运行时模块
    render_runtime_module()?;

    let found_rpc = context.has_interfaces();
    // Render all type modules
    for module in &context.modules {
        render_module(module)?;
    }

    if found_rpc {
        render_rpc_module()?;
        render_websocket_module()?;
    }

    Ok(())
}

// 渲染运行时模块
fn render_runtime_module() -> anyhow::Result<()> {
    let runtime = RuntimeTemplate {};
    let content = runtime.render().expect("Failed to render runtime module");

    let path = Path::new("Capnproto.elm");
    let mut file = File::create(path)?;
    write!(file, "{}", format_elm_code(&content))?;

    Ok(())
}

// 渲染 RPC 模块
fn render_rpc_module() -> anyhow::Result<()> {
    let rpc = RpcTemplate {};
    let content = rpc.render().expect("Failed to render runtime module");

    let path = Path::new("Rpc/Client.elm");

    // 确保目录存在
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = File::create(path)?;
    write!(file, "{}", format_elm_code(&content))?;

    Ok(())
}

// Render WebSocket module
fn render_websocket_module() -> anyhow::Result<()> {
    let websocket = WebSocketTemplate {};
    let content = websocket
        .render()
        .expect("Failed to render WebSocket module");

    let path = Path::new("Rpc/WebSocket.elm");

    // Ensure directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = File::create(path)?;
    write!(file, "{}", format_elm_code(&content))?;

    Ok(())
}

// 渲染单个模块
fn render_module(module: &ElmModule) -> anyhow::Result<()> {
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

    let path = Path::new(&file_name);

    // 确保目录存在
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // 写入文件
    let mut file = File::create(path)?;
    write!(file, "{}", format_elm_code(&content))?;

    Ok(())
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
                if let ElmType::List(_) = &field.elm_type {
                    exports.push(format!("get{}Count", field.name.to_upper_camel_case()));
                    exports.push(format!("get{}At", field.name.to_upper_camel_case()));
                } else if let ElmType::AnyPointer = &field.elm_type {
                    exports.push(format!("get{}Reader", field.name.to_upper_camel_case()));
                    exports.push(format!("get{}", field.name.to_upper_camel_case()));
                } else if let ElmType::StructRef(_, _, _) = &field.elm_type {
                    exports.push(format!("get{}Reader", field.name.to_upper_camel_case()));
                    exports.push(format!("get{}", field.name.to_upper_camel_case()));
                } else if let ElmType::UnionInline(branches, _) = &field.elm_type {
                    exports.push("Union(..)".to_owned());
                    for branch in branches {
                        if !is_void_type(&branch.elm_type) {
                            exports.push(format!("get{}", branch.name.to_upper_camel_case()));
                            if is_struct_ref(&branch.elm_type)
                                || is_anypointer_type(&branch.elm_type)
                                || is_generic_param_type(&branch.elm_type)
                            {
                                exports.push(format!(
                                    "get{}Reader",
                                    branch.name.to_upper_camel_case()
                                ));
                            }
                        }
                        exports.push(format!("is{}", branch.name.to_upper_camel_case()));
                    }
                } else if !is_void_type(&field.elm_type)
                    && elm_type_to_string(&field.elm_type) != "Union"
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
                    } else if is_void_type(&branch.elm_type) {
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

fn is_text_type(elm_type: &ElmType) -> bool {
    matches!(elm_type, ElmType::Primitive(ElmPrimitiveType::String))
}

fn is_data_type(elm_type: &ElmType) -> bool {
    matches!(elm_type, ElmType::Primitive(ElmPrimitiveType::Bytes))
}

fn is_void_type(elm_type: &ElmType) -> bool {
    matches!(elm_type, ElmType::Primitive(ElmPrimitiveType::Unit))
}

fn is_struct_ref(elm_type: &ElmType) -> bool {
    matches!(elm_type, ElmType::StructRef(_, _, _))
}

fn is_interface_ref(elm_type: &ElmType) -> bool {
    matches!(elm_type, ElmType::InterfaceRef(_, _, _))
}

fn is_pointer_type(elm_type: &ElmType) -> bool {
    match elm_type {
        ElmType::Primitive(ElmPrimitiveType::String) => true,
        ElmType::Primitive(ElmPrimitiveType::Bytes) => true,
        ElmType::Primitive(_) => false,
        ElmType::List(_) => true,
        ElmType::StructRef(_, _, _) => true,
        ElmType::EnumRef(_, _, _, _) => false,
        ElmType::InterfaceRef(_, _, _) => true,
        ElmType::AnyPointer => true,
        ElmType::UnionInline(..) => false,
        ElmType::GenericParam(_) => true,
    }
}

fn is_anypointer_type(elm_type: &ElmType) -> bool {
    matches!(elm_type, ElmType::AnyPointer)
}

fn is_generic_param_type(elm_type: &ElmType) -> bool {
    matches!(elm_type, ElmType::GenericParam(_))
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

    pub fn elm_type_str(elm_type: &ElmType) -> askama::Result<String> {
        Ok(elm_type_to_string(elm_type))
    }

    pub fn elm_type_module_str(elm_type: &ElmType) -> askama::Result<String> {
        match elm_type {
            ElmType::StructRef(m, _, _) => Ok(m.clone()),
            ElmType::UnionInline(_, _) => Ok("Union".to_owned()),
            ElmType::EnumRef(m, _, _, _) => Ok(m.clone()),
            _ => Ok("".to_owned()),
        }
    }

    pub fn parenthesize_if_needed(elm_type: &ElmType) -> askama::Result<String> {
        let s = super::elm_type_to_string(elm_type);
        if s.contains(' ') {
            Ok(format!("({})", s))
        } else {
            Ok(s)
        }
    }

    pub fn elm_write_type(elm_type: &ElmType) -> askama::Result<String> {
        let result = match elm_type {
            ElmType::Primitive(ElmPrimitiveType::Bool) => "Bool".to_owned(),
            ElmType::Primitive(ElmPrimitiveType::Int(bitwidth)) => match bitwidth {
                8 => "UInt8".to_owned(),
                16 => "UInt16".to_owned(),
                32 => "UInt32".to_owned(),
                64 => "UInt64".to_owned(),
                _ => "UInt8".to_owned(),
            },
            ElmType::Primitive(ElmPrimitiveType::Float(bitwidth)) => match bitwidth {
                32 => "Float32".to_owned(),
                _ => "Float64".to_owned(),
            },
            _ => "Int".to_owned(),
        };
        Ok(result)
    }

    pub fn elm_read_type(elm_type: &ElmType) -> askama::Result<String> {
        let result = match elm_type {
            ElmType::Primitive(ElmPrimitiveType::Bool) => "Bool".to_owned(),
            ElmType::Primitive(ElmPrimitiveType::Int(bitwidth)) => match bitwidth {
                8 => "UInt8".to_owned(),
                16 => "UInt16".to_owned(),
                32 => "UInt32".to_owned(),
                64 => "UInt64".to_owned(),
                _ => "UInt8".to_owned(),
            },
            ElmType::Primitive(ElmPrimitiveType::Float(bitwidth)) => match bitwidth {
                32 => "Float32".to_owned(),
                _ => "Float64".to_owned(),
            },
            _ => "Int".to_owned(),
        };
        Ok(result)
    }

    pub fn elm_type_bytewidth(elm_type: &ElmType) -> askama::Result<u32> {
        let result = match elm_type {
            ElmType::Primitive(ElmPrimitiveType::Bool) => 0,
            ElmType::Primitive(ElmPrimitiveType::Int(bitwidth)) => bitwidth / 8,
            ElmType::Primitive(ElmPrimitiveType::Float(bitwidth)) => bitwidth / 8,
            ElmType::Primitive(_) => 0,
            ElmType::StructRef(_, _, _) => 0,
            ElmType::EnumRef(_, _, _, _) => 0,
            ElmType::InterfaceRef(_, _, _) => 0,
            ElmType::AnyPointer => 0,
            ElmType::List(_) => 0,
            ElmType::UnionInline(_, _) => 0,
            ElmType::GenericParam(_) => 0,
        };
        Ok(result as u32)
    }

    pub fn is_boolean_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(matches!(
            elm_type,
            ElmType::Primitive(ElmPrimitiveType::Bool)
        ))
    }

    pub fn is_integer_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(matches!(
            elm_type,
            ElmType::Primitive(ElmPrimitiveType::Int(_))
        ))
    }

    pub fn is_float_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(matches!(
            elm_type,
            ElmType::Primitive(ElmPrimitiveType::Float(_))
        ))
    }

    pub fn is_text_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(super::is_text_type(elm_type))
    }

    pub fn is_data_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(super::is_data_type(elm_type))
    }

    pub fn is_struct_ref(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(super::is_struct_ref(elm_type))
    }

    pub fn is_interface_ref(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(super::is_interface_ref(elm_type))
    }

    pub fn is_enum_ref(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(matches!(elm_type, ElmType::EnumRef(_, _, _, _)))
    }

    pub fn is_list_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(matches!(elm_type, ElmType::List(_)))
    }

    pub fn is_primitive_list(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(matches!(elm_type, ElmType::List(inner) if inner.is_primitive()))
    }

    pub fn is_union_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(matches!(elm_type, ElmType::UnionInline(..)))
    }

    pub fn is_anypointer_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(super::is_anypointer_type(elm_type))
    }

    pub fn is_pointer_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(super::is_pointer_type(elm_type))
    }

    pub fn is_void_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(super::is_void_type(elm_type))
    }

    pub fn is_generic_param_type(elm_type: &ElmType) -> askama::Result<bool> {
        Ok(super::is_generic_param_type(elm_type))
    }

    // --- Default value XOR mask filters ---

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

    pub fn type_to_encoder(elm_type: &ElmType) -> askama::Result<String> {
        match elm_type {
            ElmType::Primitive(ElmPrimitiveType::String) => {
                Ok("Capnproto.textToAnyPointer".to_string())
            }
            ElmType::Primitive(ElmPrimitiveType::Bytes) => {
                Ok("Capnproto.bytesToAnyPointer".to_string())
            }
            ElmType::Primitive(ElmPrimitiveType::Int(64)) => {
                Ok("Capnproto.word64ToAnyPointer".to_string())
            }
            ElmType::StructRef(module, _, args) => {
                let mut encoders: Vec<String> = Vec::new();
                for arg in args {
                    encoders.push(type_to_encoder(arg)?);
                }
                if encoders.is_empty() {
                    Ok(format!("{}.toAnyPointer", module))
                } else {
                    Ok(encoders.join(" "))
                }
            }
            ElmType::GenericParam(_) => Ok("toAnyPointer".to_string()),
            ElmType::InterfaceRef(_, _, _) | ElmType::EnumRef(_, _, _, _) | ElmType::AnyPointer => {
                Ok("identity".to_string())
            }
            _ => Ok("Capnproto.unknownTypeEncoder".to_string()),
        }
    }

    pub fn type_to_decoder(elm_type: &ElmType) -> askama::Result<String> {
        match elm_type {
            ElmType::Primitive(ElmPrimitiveType::String) => {
                Ok("Capnproto.anyPointerToText".to_string())
            }
            ElmType::Primitive(ElmPrimitiveType::Bytes) => {
                Ok("Capnproto.anyPointerToBytes".to_string())
            }
            ElmType::Primitive(ElmPrimitiveType::Int(64)) => {
                Ok("Capnproto.anyPointerToWord64".to_string())
            }
            ElmType::StructRef(module, _, args) => {
                let mut encoders: Vec<String> = Vec::new();
                for arg in args {
                    encoders.push(type_to_encoder(arg)?);
                }
                if encoders.is_empty() {
                    Ok(format!("{}.fromAnyPointer", module))
                } else {
                    Ok(encoders.join(" "))
                }
            }
            ElmType::GenericParam(_) => Ok("fromAnyPointer".to_string()),
            ElmType::InterfaceRef(_, _, _) | ElmType::EnumRef(_, _, _, _) | ElmType::AnyPointer => {
                Ok("Just".to_string())
            }
            _ => Ok("Capnproto.unknownTypeDecoder".to_string()),
        }
    }

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
            if branch.is_pointer || is_void_type(&branch.elm_type).unwrap_or(false) {
                continue;
            }

            count += 1;
        }
        Ok(count)
    }

    pub fn branches_count(branches: &[ElmUnionBranch]) -> askama::Result<usize> {
        Ok(branches.len())
    }

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
        match elm_type {
            ElmType::List(inner) => match inner.as_ref() {
                ElmType::Primitive(ElmPrimitiveType::Int(8)) => {
                    Ok("Capnproto.encodePrimitiveIntList 1 1".to_string())
                }
                ElmType::Primitive(ElmPrimitiveType::Int(16)) => {
                    Ok("Capnproto.encodePrimitiveIntList 3 2".to_string())
                }
                ElmType::Primitive(ElmPrimitiveType::Int(32)) => {
                    Ok("Capnproto.encodePrimitiveIntList 4 4".to_string())
                }
                ElmType::Primitive(ElmPrimitiveType::Int(64)) => {
                    Ok("Capnproto.encodePrimitiveIntList 5 8".to_string())
                }
                ElmType::Primitive(ElmPrimitiveType::Bool) => {
                    Ok("Capnproto.encodeBoolList".to_string())
                }
                ElmType::Primitive(ElmPrimitiveType::Float(32)) => {
                    Ok("Capnproto.encodeFloat32List".to_string())
                }
                ElmType::Primitive(ElmPrimitiveType::Float(64)) => {
                    Ok("Capnproto.encodeFloat64List".to_string())
                }
                ElmType::Primitive(ElmPrimitiveType::String) => {
                    Ok("Capnproto.encodeTextList".to_string())
                }
                ElmType::StructRef(module_name, _, _) => {
                    if module_name.is_empty() {
                        Ok("Capnproto.encodeStructList encode dataWords pointerWords".to_string())
                    } else {
                        Ok(format!(
                            "Capnproto.encodeStructList {}.encode {}.dataWords {}.pointerWords",
                            module_name, module_name, module_name
                        ))
                    }
                }
                _ => Ok("Capnproto.encodeStructList encode dataWords pointerWords".to_string()),
            },
            _ => Ok("Capnproto.encodeStructList encode dataWords pointerWords".to_string()),
        }
    }

    /// Returns the list element decoder expression for getXAt functions
    pub fn list_element_reader_expr(elm_type: &ElmType) -> askama::Result<String> {
        match elm_type {
            ElmType::List(inner) => match inner.as_ref() {
                ElmType::Primitive(ElmPrimitiveType::Int(8)) => {
                    Ok("\\r -> Capnproto.readUInt8 r 0".to_string())
                }
                ElmType::Primitive(ElmPrimitiveType::Int(16)) => {
                    Ok("\\r -> Capnproto.readUInt16 r 0".to_string())
                }
                ElmType::Primitive(ElmPrimitiveType::Int(32)) => {
                    Ok("\\r -> Capnproto.readUInt32 r 0".to_string())
                }
                ElmType::Primitive(ElmPrimitiveType::Int(64)) => {
                    Ok("\\r -> Capnproto.readUInt64 r 0".to_string())
                }
                ElmType::Primitive(ElmPrimitiveType::Bool) => {
                    Ok("\\r -> Capnproto.readBool r 0 0".to_string())
                }
                ElmType::Primitive(ElmPrimitiveType::Float(32)) => {
                    Ok("\\r -> Capnproto.readFloat32 r 0".to_string())
                }
                ElmType::Primitive(ElmPrimitiveType::Float(64)) => {
                    Ok("\\r -> Capnproto.readFloat64 r 0".to_string())
                }
                ElmType::Primitive(ElmPrimitiveType::String) => {
                    Ok("\\r -> Capnproto.readText r 0".to_string())
                }
                ElmType::StructRef(module_name, _, _) => {
                    if module_name.is_empty() {
                        Ok("\\r -> decode r |> Result.toMaybe".to_string())
                    } else {
                        Ok(format!("\\r -> {}.decode r |> Result.toMaybe", module_name))
                    }
                }
                ElmType::EnumRef(module_name, _, _, _) => {
                    if module_name.is_empty() {
                        Ok("\\r -> Capnproto.readUInt16 r 0".to_string())
                    } else {
                        Ok(format!(
                            "\\r -> Capnproto.readUInt16 r 0 |> Maybe.andThen {}.fromCode",
                            module_name
                        ))
                    }
                }
                _ => Ok("\\r -> Nothing".to_string()),
            },
            _ => Ok("\\r -> Nothing".to_string()),
        }
    }
}
fn elm_type_to_string(elm_type: &ElmType) -> String {
    match elm_type {
        ElmType::Primitive(ElmPrimitiveType::Bool) => "Bool".to_owned(),
        ElmType::Primitive(ElmPrimitiveType::Int(64)) => "Capnproto.Word64".to_owned(),
        ElmType::Primitive(ElmPrimitiveType::Int(_)) => "Int".to_owned(),
        ElmType::Primitive(ElmPrimitiveType::Float(_)) => "Float".to_owned(),
        ElmType::Primitive(ElmPrimitiveType::String) => "String".to_owned(),
        ElmType::Primitive(ElmPrimitiveType::Bytes) => "Bytes.Bytes".to_owned(),
        ElmType::Primitive(ElmPrimitiveType::Unit) => "".to_owned(),
        ElmType::AnyPointer => "Capnproto.AnyPointer".to_owned(),
        ElmType::InterfaceRef(_, _, _) => "Rpc.Capability".to_owned(),
        ElmType::List(inner) => format!("List ({})", elm_type_to_string(inner)),
        ElmType::StructRef(m, s, args) => {
            let base = if m.is_empty() {
                s.to_string()
            } else {
                format!("{}.{}", m, s)
            };
            render_with_args(&base, args)
        }
        ElmType::EnumRef(m, e, _, args) => {
            let base = if m.is_empty() {
                e.to_string()
            } else {
                format!("{}.{}", m, e)
            };
            render_with_args(&base, args)
        }
        ElmType::UnionInline(_, gps) => {
            if gps.is_empty() {
                "Union".to_owned()
            } else if gps.len() > 1 {
                format!("(Union {})", gps.join(" "))
            } else {
                format!("Union {}", gps.join(""))
            }
        }
        ElmType::GenericParam(name) => name.clone(),
    }
}

fn render_with_args(base: &str, args: &[ElmType]) -> String {
    if args.is_empty() {
        base.to_string()
    } else {
        let args_str = args
            .iter()
            .map(elm_type_to_string)
            .collect::<Vec<_>>()
            .join(" ");
        format!("{} {}", base, args_str)
    }
}

/// Attempts to format Elm code using the elm-format command.
///
/// If elm-format is not found or fails, it returns the original code
/// and prints a warning to stderr.
fn format_elm_code(unformatted_code: &str) -> String {
    match std::process::Command::new("elm-format")
        .arg("--yes")
        .arg("--stdin")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(mut child) => {
            // Get stdin handle and write the unformatted code
            if let Some(mut stdin) = child.stdin.take() {
                if stdin.write_all(unformatted_code.as_bytes()).is_err() {
                    return unformatted_code.to_string();
                }
            } else {
                return unformatted_code.to_string();
            }

            // Wait for the process to finish and get output
            match child.wait_with_output() {
                Ok(output) => {
                    if output.status.success() {
                        match String::from_utf8(output.stdout) {
                            Ok(formatted_code) => formatted_code,
                            Err(_) => unformatted_code.to_string(),
                        }
                    } else {
                        let _ = String::from_utf8_lossy(&output.stderr);
                        unformatted_code.to_string()
                    }
                }
                Err(_) => unformatted_code.to_string(),
            }
        }
        Err(_) => unformatted_code.to_string(),
    }
}
