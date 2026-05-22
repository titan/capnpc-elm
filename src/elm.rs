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

impl ElmType {
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
