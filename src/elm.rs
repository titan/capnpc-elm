type ModuleName = String;
type EntityName = &'static str;

/// Cap'n Proto 默认值（用于 data section 字段的 XOR mask）
#[derive(Debug, Clone)]
pub enum ElmDefaultValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Enum(u16),
}

impl ElmDefaultValue {
    /// 是否为零值（零值不需要 XOR）
    pub fn is_zero(&self) -> bool {
        match self {
            ElmDefaultValue::Bool(b) => !b,
            ElmDefaultValue::Int(i) => *i == 0,
            ElmDefaultValue::Float(f) => *f == 0.0,
            ElmDefaultValue::Enum(v) => *v == 0,
        }
    }

    /// 渲染为 Elm 表达式字面量
    pub fn to_elm_literal(&self) -> String {
        match self {
            ElmDefaultValue::Bool(true) => "True".to_string(),
            ElmDefaultValue::Bool(false) => "False".to_string(),
            ElmDefaultValue::Int(i) => {
                // 使用十六进制确保负数在 Elm 中正确表示
                format!("{}", i)
            }
            ElmDefaultValue::Float(f) => {
                // Elm 要求浮点字面量包含小数点
                let s = format!("{}", f);
                if !s.contains('.') {
                    format!("{}.0", s)
                } else {
                    s
                }
            }
            ElmDefaultValue::Enum(v) => format!("{}", v),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ElmPrimitiveType {
    Bool,
    Int(u8),
    Float(u8),
    String,
    Bytes,
    Unit,
}

#[derive(Debug, Clone)]
pub enum ElmType {
    Primitive(ElmPrimitiveType),
    StructRef(ModuleName, EntityName, Vec<ElmType>),
    EnumRef(ModuleName, EntityName, Vec<ElmEnumVariant>, Vec<ElmType>),
    InterfaceRef(ModuleName, EntityName, Vec<ElmType>),
    AnyPointer,
    List(Box<ElmType>),
    UnionInline(Vec<ElmUnionBranch>, Vec<String>), // ([Branch], [GenericParam])
    GenericParam(String),
}

/// 辅助函数：为带泛型参数的类型渲染类型应用表达式
fn render_with_args(base: &str, args: &[ElmType]) -> String {
    if args.is_empty() {
        base.to_string()
    } else {
        let args_str = args
            .iter()
            .map(|a| a.to_elm_string())
            .collect::<Vec<_>>()
            .join(" ");
        format!("{} {}", base, args_str)
    }
}

impl ElmType {
    // ── 原有方法 ──────────────────────────────────────────

    pub fn is_primitive(&self) -> bool {
        match self {
            ElmType::Primitive(_) => true,
            ElmType::List(_) => false,
            ElmType::StructRef(_, _, _)
            | ElmType::EnumRef(_, _, _, _)
            | ElmType::InterfaceRef(_, _, _)
            | ElmType::UnionInline(_, _)
            | ElmType::GenericParam(_)
            | ElmType::AnyPointer => false,
        }
    }

    /// Check if this type (recursively) contains an InterfaceRef (capability).
    /// Used to determine if a struct needs cap table handling in RPC payloads.
    pub fn contains_interface_ref(&self) -> bool {
        match self {
            ElmType::InterfaceRef(_, _, _) => true,
            ElmType::StructRef(_, _, type_args) => {
                type_args.iter().any(|arg| arg.contains_interface_ref())
            }
            ElmType::EnumRef(_, _, _, type_args) => {
                type_args.iter().any(|arg| arg.contains_interface_ref())
            }
            ElmType::List(inner) => inner.contains_interface_ref(),
            ElmType::UnionInline(branches, _) => {
                branches.iter().any(|b| b.elm_type.contains_interface_ref())
            }
            ElmType::GenericParam(_) | ElmType::AnyPointer | ElmType::Primitive(_) => false,
        }
    }

    /// Collect all InterfaceRef module names found (recursively) in this type.
    /// Used for generating capability-related imports in interface methods.
    pub fn collect_interface_refs(&self) -> Vec<String> {
        match self {
            ElmType::InterfaceRef(module_name, _, _) => {
                if module_name.is_empty() {
                    vec![]
                } else {
                    vec![module_name.clone()]
                }
            }
            ElmType::StructRef(_, _, type_args) => type_args
                .iter()
                .flat_map(|arg| arg.collect_interface_refs())
                .collect(),
            ElmType::EnumRef(_, _, _, type_args) => type_args
                .iter()
                .flat_map(|arg| arg.collect_interface_refs())
                .collect(),
            ElmType::List(inner) => inner.collect_interface_refs(),
            ElmType::UnionInline(branches, _) => branches
                .iter()
                .flat_map(|b| b.elm_type.collect_interface_refs())
                .collect(),
            ElmType::GenericParam(_) | ElmType::AnyPointer | ElmType::Primitive(_) => vec![],
        }
    }

    // ── 类型谓词（从 render.rs 自由函数迁移） ──────────────

    pub fn is_text(&self) -> bool {
        matches!(self, ElmType::Primitive(ElmPrimitiveType::String))
    }

    pub fn is_data(&self) -> bool {
        matches!(self, ElmType::Primitive(ElmPrimitiveType::Bytes))
    }

    pub fn is_void(&self) -> bool {
        matches!(self, ElmType::Primitive(ElmPrimitiveType::Unit))
    }

    pub fn is_struct_ref(&self) -> bool {
        matches!(self, ElmType::StructRef(_, _, _))
    }

    pub fn is_interface_ref(&self) -> bool {
        matches!(self, ElmType::InterfaceRef(_, _, _))
    }

    pub fn is_pointer(&self) -> bool {
        match self {
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

    pub fn is_anypointer(&self) -> bool {
        matches!(self, ElmType::AnyPointer)
    }

    pub fn is_generic_param(&self) -> bool {
        matches!(self, ElmType::GenericParam(_))
    }

    pub fn is_enum_ref(&self) -> bool {
        matches!(self, ElmType::EnumRef(_, _, _, _))
    }

    pub fn is_list(&self) -> bool {
        matches!(self, ElmType::List(_))
    }

    pub fn is_primitive_list(&self) -> bool {
        matches!(self, ElmType::List(inner) if inner.is_primitive())
    }

    pub fn is_union(&self) -> bool {
        matches!(self, ElmType::UnionInline(..))
    }

    pub fn is_boolean(&self) -> bool {
        matches!(self, ElmType::Primitive(ElmPrimitiveType::Bool))
    }

    pub fn is_integer(&self) -> bool {
        matches!(self, ElmType::Primitive(ElmPrimitiveType::Int(_)))
    }

    pub fn is_float(&self) -> bool {
        matches!(self, ElmType::Primitive(ElmPrimitiveType::Float(_)))
    }

    // ── 类型字符串表示（从 render.rs 迁移） ────────────────

    /// 渲染为 Elm 类型表达式字符串
    pub fn to_elm_string(&self) -> String {
        match self {
            ElmType::Primitive(ElmPrimitiveType::Bool) => "Bool".to_owned(),
            ElmType::Primitive(ElmPrimitiveType::Int(64)) => "Capnproto.Word64".to_owned(),
            ElmType::Primitive(ElmPrimitiveType::Int(_)) => "Int".to_owned(),
            ElmType::Primitive(ElmPrimitiveType::Float(_)) => "Float".to_owned(),
            ElmType::Primitive(ElmPrimitiveType::String) => "String".to_owned(),
            ElmType::Primitive(ElmPrimitiveType::Bytes) => "Bytes.Bytes".to_owned(),
            ElmType::Primitive(ElmPrimitiveType::Unit) => "".to_owned(),
            ElmType::AnyPointer => "Capnproto.AnyPointer".to_owned(),
            ElmType::InterfaceRef(_, _, _) => "Rpc.Capability".to_owned(),
            ElmType::List(inner) => format!("List ({})", inner.to_elm_string()),
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

    /// 返回所属模块名（用于 import 和限定引用）
    pub fn module_name(&self) -> String {
        match self {
            ElmType::StructRef(m, _, _) => m.clone(),
            ElmType::UnionInline(_, _) => "Union".to_owned(),
            ElmType::EnumRef(m, _, _, _) => m.clone(),
            _ => "".to_owned(),
        }
    }

    // ── 写入/读取类型名（从 render.rs filters 迁移） ────────

    /// 返回写入操作使用的原始类型名（UInt8, Float32 等）
    pub fn write_type(&self) -> String {
        match self {
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
        }
    }

    /// 返回读取操作使用的原始类型名（UInt8, Float32 等）
    pub fn read_type(&self) -> String {
        // 当前 write_type 和 read_type 逻辑一致
        self.write_type()
    }

    /// 返回该类型在 data section 中占用的字节数
    pub fn byte_width(&self) -> u32 {
        match self {
            ElmType::Primitive(ElmPrimitiveType::Bool) => 0,
            ElmType::Primitive(ElmPrimitiveType::Int(bitwidth)) => (*bitwidth / 8) as u32,
            ElmType::Primitive(ElmPrimitiveType::Float(bitwidth)) => (*bitwidth / 8) as u32,
            ElmType::Primitive(_) => 0,
            ElmType::StructRef(_, _, _) => 0,
            ElmType::EnumRef(_, _, _, _) => 0,
            ElmType::InterfaceRef(_, _, _) => 0,
            ElmType::AnyPointer => 0,
            ElmType::List(_) => 0,
            ElmType::UnionInline(_, _) => 0,
            ElmType::GenericParam(_) => 0,
        }
    }

    // ── 编解码表达式（从 render.rs filters 迁移） ──────────

    /// 返回将该类型编码到 AnyPointer 的 Elm 表达式（函数名或组合子）
    pub fn encoder_expr(&self) -> String {
        match self {
            ElmType::Primitive(ElmPrimitiveType::String) => {
                "Capnproto.textToAnyPointer".to_string()
            }
            ElmType::Primitive(ElmPrimitiveType::Bytes) => {
                "Capnproto.bytesToAnyPointer".to_string()
            }
            ElmType::Primitive(ElmPrimitiveType::Int(64)) => {
                "Capnproto.word64ToAnyPointer".to_string()
            }
            ElmType::StructRef(module, _, args) => {
                let encoders: Vec<String> = args.iter().map(|a| a.encoder_expr()).collect();
                if encoders.is_empty() {
                    format!("{}.toAnyPointer", module)
                } else {
                    encoders.join(" ")
                }
            }
            ElmType::GenericParam(_) => "toAnyPointer".to_string(),
            ElmType::InterfaceRef(_, _, _) | ElmType::EnumRef(_, _, _, _) | ElmType::AnyPointer => {
                "identity".to_string()
            }
            _ => "Capnproto.unknownTypeEncoder".to_string(),
        }
    }

    /// 返回从 AnyPointer 解码该类型的 Elm 表达式（函数名或组合子）
    pub fn decoder_expr(&self) -> String {
        match self {
            ElmType::Primitive(ElmPrimitiveType::String) => {
                "Capnproto.anyPointerToText".to_string()
            }
            ElmType::Primitive(ElmPrimitiveType::Bytes) => {
                "Capnproto.anyPointerToBytes".to_string()
            }
            ElmType::Primitive(ElmPrimitiveType::Int(64)) => {
                "Capnproto.anyPointerToWord64".to_string()
            }
            ElmType::StructRef(module, _, args) => {
                // 解码时对泛型参数仍使用 encoder（toAnyPointer 方向）
                let encoders: Vec<String> = args.iter().map(|a| a.encoder_expr()).collect();
                if encoders.is_empty() {
                    format!("{}.fromAnyPointer", module)
                } else {
                    encoders.join(" ")
                }
            }
            ElmType::GenericParam(_) => "fromAnyPointer".to_string(),
            ElmType::InterfaceRef(_, _, _) | ElmType::EnumRef(_, _, _, _) | ElmType::AnyPointer => {
                "Just".to_string()
            }
            _ => "Capnproto.unknownTypeDecoder".to_string(),
        }
    }

    /// 返回列表编码器的完整 Elm 表达式
    pub fn list_encoder_expr(&self) -> String {
        match self {
            ElmType::List(inner) => match inner.as_ref() {
                ElmType::Primitive(ElmPrimitiveType::Int(8)) => {
                    "Capnproto.encodePrimitiveIntList 1 1".to_string()
                }
                ElmType::Primitive(ElmPrimitiveType::Int(16)) => {
                    "Capnproto.encodePrimitiveIntList 3 2".to_string()
                }
                ElmType::Primitive(ElmPrimitiveType::Int(32)) => {
                    "Capnproto.encodePrimitiveIntList 4 4".to_string()
                }
                ElmType::Primitive(ElmPrimitiveType::Int(64)) => {
                    "Capnproto.encodePrimitiveIntList 5 8".to_string()
                }
                ElmType::Primitive(ElmPrimitiveType::Bool) => {
                    "Capnproto.encodeBoolList".to_string()
                }
                ElmType::Primitive(ElmPrimitiveType::Float(32)) => {
                    "Capnproto.encodeFloat32List".to_string()
                }
                ElmType::Primitive(ElmPrimitiveType::Float(64)) => {
                    "Capnproto.encodeFloat64List".to_string()
                }
                ElmType::Primitive(ElmPrimitiveType::String) => {
                    "Capnproto.encodeTextList".to_string()
                }
                ElmType::StructRef(module_name, _, _) => {
                    if module_name.is_empty() {
                        "Capnproto.encodeStructList encode dataWords pointerWords".to_string()
                    } else {
                        format!(
                            "Capnproto.encodeStructList {}.encode {}.dataWords {}.pointerWords",
                            module_name, module_name, module_name
                        )
                    }
                }
                _ => "Capnproto.encodeStructList encode dataWords pointerWords".to_string(),
            },
            _ => "Capnproto.encodeStructList encode dataWords pointerWords".to_string(),
        }
    }

    /// 返回列表元素读取器的 Elm lambda 表达式
    pub fn list_element_reader_expr(&self) -> String {
        match self {
            ElmType::List(inner) => match inner.as_ref() {
                ElmType::Primitive(ElmPrimitiveType::Int(8)) => {
                    "\\r -> Capnproto.readUInt8 r 0".to_string()
                }
                ElmType::Primitive(ElmPrimitiveType::Int(16)) => {
                    "\\r -> Capnproto.readUInt16 r 0".to_string()
                }
                ElmType::Primitive(ElmPrimitiveType::Int(32)) => {
                    "\\r -> Capnproto.readUInt32 r 0".to_string()
                }
                ElmType::Primitive(ElmPrimitiveType::Int(64)) => {
                    "\\r -> Capnproto.readUInt64 r 0".to_string()
                }
                ElmType::Primitive(ElmPrimitiveType::Bool) => {
                    "\\r -> Capnproto.readBool r 0 0".to_string()
                }
                ElmType::Primitive(ElmPrimitiveType::Float(32)) => {
                    "\\r -> Capnproto.readFloat32 r 0".to_string()
                }
                ElmType::Primitive(ElmPrimitiveType::Float(64)) => {
                    "\\r -> Capnproto.readFloat64 r 0".to_string()
                }
                ElmType::Primitive(ElmPrimitiveType::String) => {
                    "\\r -> Capnproto.readText r 0".to_string()
                }
                ElmType::StructRef(module_name, _, _) => {
                    if module_name.is_empty() {
                        "\\r -> decode r |> Result.toMaybe".to_string()
                    } else {
                        format!("\\r -> {}.decode r |> Result.toMaybe", module_name)
                    }
                }
                ElmType::EnumRef(module_name, _, _, _) => {
                    if module_name.is_empty() {
                        "\\r -> Capnproto.readUInt16 r 0".to_string()
                    } else {
                        format!(
                            "\\r -> Capnproto.readUInt16 r 0 |> Maybe.andThen {}.fromCode",
                            module_name
                        )
                    }
                }
                _ => "\\r -> Nothing".to_string(),
            },
            _ => "\\r -> Nothing".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ElmModule {
    pub id: u64,
    pub name: String, // Module name
    pub path: String, // Module path
    pub imports: Vec<String>,
    pub type_def: ElmTypeDef,
    pub data_words: u32,
    pub pointer_words: u32,
    pub fields: Vec<ElmField>,
    pub discriminant_offset: u32, // Union discriminant offset in bytes
    pub methods: Vec<ElmMethod>,
    pub generic_params: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum ElmTypeDef {
    Struct,
    Enum,
    Interface,
}

#[derive(Debug, Clone)]
pub struct ElmField {
    pub name: String,
    pub discriminant: Option<u16>,
    pub elm_type: ElmType,
    pub offset: u32,              // Offset for data fields
    pub is_union_container: bool, // Whether this field is a union container
    pub default_value: Option<ElmDefaultValue>,
}

#[derive(Debug, Clone)]
pub struct ElmUnionBranch {
    pub name: String,
    pub discriminant: u16,
    pub elm_type: ElmType,
    pub offset: u32,      // Field offset within the struct
    pub is_pointer: bool, // Whether this field is a pointer
    pub default_value: Option<ElmDefaultValue>,
}

#[derive(Debug, Clone)]
pub struct ElmEnumVariant {
    pub name: String,
    pub ordinal: u16,
}

#[derive(Debug, Clone)]
pub struct ElmMethod {
    pub id: u16,
    pub name: String,
    pub implicit_parameters: Vec<String>,
    pub param_type: ElmType,
    pub result_type: ElmType,
    /// Whether the param type contains interface refs (capabilities).
    /// When true, the generated call code must build a capTable.
    pub param_has_caps: bool,
    /// Whether the result type contains interface refs (capabilities).
    /// When true, the generated return handler must process received CapDescriptors.
    pub result_has_caps: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ElmInterface {
    pub id: u64,
    pub name: String,
    pub path: String,
    pub methods: Vec<ElmMethod>,
}

#[derive(Debug, Clone)]
pub struct ElmContext {
    pub modules: Vec<ElmModule>,
}

impl ElmContext {
    pub fn new() -> Self {
        ElmContext {
            modules: Vec::new(),
        }
    }

    /// Check if any module in the context is an interface (has methods).
    pub fn has_interfaces(&self) -> bool {
        self.modules
            .iter()
            .any(|m| matches!(m.type_def, ElmTypeDef::Interface))
    }
}
