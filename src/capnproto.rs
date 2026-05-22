use anyhow::{Context, Result};
use capnp::schema_capnp::code_generator_request;
use capnp::schema_capnp::{field, node, type_};
use std::collections::{HashMap, VecDeque};
use std::fmt::Debug;

type NodeId = u64;
type ScopeId = u64;
type FileId = u64;

#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    pub display_name: String,
    pub kind: NodeKind,
    pub nested_nodes: Vec<Node>,
    pub parent_id: ScopeId,
    pub file_id: FileId,
    pub generic_params: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum NodeKind {
    Struct {
        is_group: bool,
        data_word_count: u16,
        pointer_word_count: u16,
        fields: Vec<Field>,
        union_fields: Option<Vec<Field>>,
        discriminant_offset: u32,
    },
    Enum(Vec<Enumerator>),
    Interface(Vec<Method>),
    Const,
    Annotation(ScopeId, NodeId),
    File,
    Other,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub discriminant: Option<u16>,
    pub typ: Type,
    pub offset: u32,
    pub default_value: Option<DefaultValue>,
}

/// Cap'n Proto 默认值（仅限 data section 字段类型）
#[derive(Debug, Clone)]
pub enum DefaultValue {
    Bool(bool),
    Int8(i8),
    Int16(i16),
    Int32(i32),
    Int64(i64),
    UInt8(u8),
    UInt16(u16),
    UInt32(u32),
    UInt64(u64),
    Float32(f32),
    Float64(f64),
    Enum(u16),
}

#[derive(Debug, Clone)]
pub struct Enumerator {
    pub name: String,
    pub ordinal: u16,
}

#[derive(Debug, Clone)]
pub struct Method {
    pub id: u16,
    pub name: String,
    pub implicit_parameters: Vec<String>,
    pub param_type: Type,
    pub result_type: Type,
}

#[derive(Debug, Clone)]
pub enum Type {
    Void,
    Bool,
    Int8,
    Int16,
    Int32,
    Int64,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    Float32,
    Float64,
    Text,
    Data,
    List(Box<Type>),
    StructRef(u64, Brand),
    EnumRef(u64, Brand),
    InterfaceRef(u64, Brand),
    AnyPointer,
    GenericParam(u16),
}

impl Type {
    pub fn get_ref_id(&self) -> Option<u64> {
        match self {
            Type::StructRef(id, _) => Some(*id),
            Type::EnumRef(id, _) => Some(*id),
            Type::InterfaceRef(id, _) => Some(*id),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Brand {
    pub scopes: Vec<BrandScope>,
}

#[derive(Debug, Clone)]
pub enum BrandScope {
    Bind(Vec<Binding>),
    Inherit,
}

#[derive(Debug, Clone)]
pub enum Binding {
    Unbound,
    Type(Type),
}

#[derive(Debug)]
pub struct RequestedFile {
    pub id: u64,
    pub filename: String,
}

/// parse_node 的命名返回值，替代无名元组
struct ParseNodeResult {
    node: Node,
    nested_node_ids: Vec<NodeId>,
}

/// parse_schema 的命名返回值，替代无名元组
pub struct ParsedSchema {
    pub nodes: Vec<Node>,
    pub requested_files: Vec<RequestedFile>,
}

pub fn parse_schema(
    message_reader: &capnp::message::Reader<capnp::serialize::OwnedSegments>,
) -> anyhow::Result<ParsedSchema> {
    let request = message_reader.get_root::<code_generator_request::Reader>()?;

    // 第一遍：解析所有节点并记录父子关系
    let (all_nodes, children_map) = collect_flat_nodes(&request)?;

    // 解析请求的文件
    let requested_files = collect_requested_files(&request)?;

    // 构建节点到文件的映射（BFS 遍历）
    let node_to_file_map = resolve_file_ids(&requested_files, &children_map);

    // 为节点设置 file_id
    let mut all_nodes = all_nodes;
    for node in all_nodes.values_mut() {
        node.file_id = *node_to_file_map.get(&node.id).unwrap_or(&0);
    }

    // 第二遍：构建嵌套结构
    let root_nodes = build_nested_structure(&mut all_nodes, &children_map, 0);

    // 第三遍：过滤无用节点
    let nodes = filter_root_nodes(root_nodes);

    Ok(ParsedSchema {
        nodes,
        requested_files,
    })
}

/// 解析请求的文件列表
fn collect_requested_files(
    request: &code_generator_request::Reader,
) -> Result<Vec<RequestedFile>> {
    let files_reader = request.get_requested_files()?;
    let mut requested_files = Vec::with_capacity(files_reader.len() as usize);
    for i in 0..files_reader.len() {
        let file_reader = files_reader.get(i);
        let id = file_reader.get_id();
        let filename = file_reader.get_filename()?.to_string()?;
        requested_files.push(RequestedFile { id, filename });
    }
    Ok(requested_files)
}

/// 第一遍扫描：解析所有平面节点，记录父子关系和嵌套节点映射
fn collect_flat_nodes(
    request: &code_generator_request::Reader,
) -> Result<(HashMap<u64, Node>, HashMap<u64, Vec<u64>>)> {
    let nodes_reader = request.get_nodes()?;
    let mut all_nodes: HashMap<u64, Node> = HashMap::with_capacity(nodes_reader.len() as usize);
    let mut children_map: HashMap<u64, Vec<u64>> =
        HashMap::with_capacity(nodes_reader.len() as usize);
    let mut nested_nodes_map = HashMap::new();

    for i in 0..nodes_reader.len() {
        let node_reader = nodes_reader.get(i);
        let id = node_reader.get_id();
        let scope_id = node_reader.get_scope_id();

        let ParseNodeResult {
            mut node,
            nested_node_ids,
        } = parse_node(node_reader)?;
        node.parent_id = scope_id;

        nested_nodes_map.insert(id, nested_node_ids);

        // 过滤注解节点
        match &node.kind {
            NodeKind::Annotation(_, _) => {
                continue; // 跳过注解节点
            }
            _ => {
                children_map.entry(scope_id).or_default().push(id);
                all_nodes.insert(id, node);
            }
        }
    }

    // 用于显式设置方法参数/结果结构体的父节点
    for (&parent_id, nested_ids) in &nested_nodes_map {
        for &child_id in nested_ids {
            if let Some(child_node) = all_nodes.get_mut(&child_id) {
                if child_node.parent_id == 0 {
                    child_node.parent_id = parent_id;
                    // 更新children_map
                    children_map.entry(parent_id).or_default().push(child_id);
                }
            }
        }
    }

    Ok((all_nodes, children_map))
}

/// BFS 遍历，为每个节点解析其所属文件 ID
fn resolve_file_ids(
    requested_files: &[RequestedFile],
    children_map: &HashMap<u64, Vec<u64>>,
) -> HashMap<u64, FileId> {
    let mut node_to_file_map: HashMap<u64, FileId> = HashMap::new();

    // 文件根节点映射到自己
    for file in requested_files {
        node_to_file_map.insert(file.id, file.id);
    }

    // BFS 遍历所有节点，继承父节点的文件 ID
    let mut queue: VecDeque<(u64, u64)> = VecDeque::new(); // (node_id, parent_id)
    for file in requested_files {
        if let Some(children) = children_map.get(&file.id) {
            for &child_id in children {
                queue.push_back((child_id, file.id));
            }
        }
    }

    while let Some((node_id, parent_id)) = queue.pop_front() {
        if let Some(&file_id) = node_to_file_map.get(&parent_id) {
            node_to_file_map.insert(node_id, file_id);
        }
        if let Some(children) = children_map.get(&node_id) {
            for &child_id in children {
                queue.push_back((child_id, node_id));
            }
        }
    }

    node_to_file_map
}

/// 第三遍：过滤掉 File/Annotation/Other 等无用根节点
fn filter_root_nodes(root_nodes: Vec<Node>) -> Vec<Node> {
    let mut nodes = Vec::new();
    for n in root_nodes {
        match n.kind {
            NodeKind::Annotation(_, _) => continue,
            NodeKind::File => {
                for nn in n.nested_nodes {
                    nodes.push(nn);
                }
            }
            NodeKind::Other => continue,
            _ => nodes.push(n),
        }
    }
    nodes
}

fn build_nested_structure(
    node_map: &mut HashMap<u64, Node>,
    children_map: &HashMap<u64, Vec<u64>>,
    parent_id: u64,
) -> Vec<Node> {
    // 直接过滤并处理子节点
    children_map
        .get(&parent_id)
        .into_iter()
        .flatten()
        .filter_map(|&child_id| {
            // 移除并过滤掉 id 为 0 的节点
            let mut node = node_map.remove(&child_id)?;
            if node.id == 0 {
                return None;
            }

            // 递归构建子节点的嵌套结构
            node.nested_nodes = build_nested_structure(node_map, children_map, child_id);
            Some(node)
        })
        .collect()
}

fn parse_node(reader: capnp::schema_capnp::node::Reader) -> anyhow::Result<ParseNodeResult> {
    let id = reader.get_id();
    let scope_id = reader.get_scope_id();
    let display_name = reader.get_display_name()?.to_string()?;
    let mut nested_node_ids = Vec::new();

    // 解析泛型参数
    let mut generic_params = Vec::new();
    let parameters_reader = reader.get_parameters()?;
    for i in 0..parameters_reader.len() {
        let param_reader = parameters_reader.get(i);
        let param_name = param_reader.get_name()?.to_string()?;
        generic_params.push(param_name);
    }

    let kind = match reader.which()? {
        node::Struct(struct_reader) => {
            let fields_reader = struct_reader.get_fields()?;

            // 收集所有字段
            let mut all_fields = Vec::new();

            for i in 0..fields_reader.len() {
                let field_reader = fields_reader.get(i);
                let field = parse_field(field_reader)?;
                all_fields.push(field);
            }

            let discriminant_offset = struct_reader.get_discriminant_offset();
            let mut normal_fields = Vec::new();
            let mut union_fields = Vec::new();

            // 分离普通字段和联合体字段
            for field in all_fields {
                if field.discriminant.is_some() {
                    union_fields.push(field);
                } else {
                    normal_fields.push(field);
                }
            }

            // 如果有联合体字段，则创建联合体部分
            let union_fields = if !union_fields.is_empty() {
                Some(union_fields)
            } else {
                None
            };

            NodeKind::Struct {
                is_group: struct_reader.get_is_group(),
                data_word_count: struct_reader.get_data_word_count(),
                pointer_word_count: struct_reader.get_pointer_count(),
                fields: normal_fields,
                union_fields,
                discriminant_offset,
            }
        }
        node::Enum(enum_reader) => {
            let enumerants_reader = enum_reader.get_enumerants()?;
            let mut enumerants = Vec::new();

            for i in 0..enumerants_reader.len() {
                let enumerant_reader = enumerants_reader.get(i);
                enumerants.push(parse_enumerant(enumerant_reader)?);
            }

            NodeKind::Enum(enumerants)
        }
        node::Interface(interface_reader) => {
            let methods_reader = interface_reader.get_methods()?;
            let mut methods = Vec::with_capacity(methods_reader.len() as usize);

            for i in 0..methods_reader.len() {
                let method_reader = methods_reader.get(i);
                let name = method_reader.get_name()?.to_string()?;

                // 解析隐式参数
                let mut implicit_parameters = Vec::new();
                let params_reader = method_reader.get_implicit_parameters()?;
                for j in 0..params_reader.len() {
                    let param = params_reader.get(j);
                    implicit_parameters.push(param.get_name()?.to_string()?);
                }

                let param_type_id = method_reader.get_param_struct_type();
                let result_type_id = method_reader.get_result_struct_type();

                nested_node_ids.push(param_type_id);
                nested_node_ids.push(result_type_id);

                methods.push(Method {
                    id: method_reader.get_code_order(),
                    name,
                    implicit_parameters,
                    param_type: Type::StructRef(
                        param_type_id,
                        parse_brand(method_reader.get_param_brand()?)?,
                    ),
                    result_type: Type::StructRef(
                        result_type_id,
                        parse_brand(method_reader.get_result_brand()?)?,
                    ),
                });
            }
            NodeKind::Interface(methods)
        }
        node::Const(_) => NodeKind::Const,
        node::Annotation(_) => NodeKind::Annotation(scope_id, id),
        node::File(()) => NodeKind::File,
    };

    // 解析嵌套节点列表
    let nested_nodes_reader = reader.get_nested_nodes()?;
    for i in 0..nested_nodes_reader.len() {
        nested_node_ids.push(nested_nodes_reader.get(i).get_id());
    }

    Ok(ParseNodeResult {
        node: Node {
            id,
            display_name,
            kind,
            nested_nodes: Vec::new(), // 将在后续步骤中填充
            parent_id: scope_id,
            file_id: 0, // 将在后续步骤中填充
            generic_params,
        },
        nested_node_ids, // 返回嵌套节点ID列表
    })
}

fn parse_field(reader: capnp::schema_capnp::field::Reader) -> anyhow::Result<Field> {
    let name = reader.get_name()?.to_string()?;
    let discriminant_value = reader.get_discriminant_value();
    let discriminant = if discriminant_value != 0xFFFF {
        Some(discriminant_value)
    } else {
        None
    };

    let (offset, typ, default_value) = match reader.which()? {
        field::Slot(slot) => {
            let offset = slot.get_offset();
            let type_reader = slot.get_type()?;
            let typ = parse_type(type_reader)?;

            let default_value = if slot.get_had_explicit_default() {
                extract_default_value(&slot)
            } else {
                None
            };

            (offset, typ, default_value)
        }
        field::Group(group) => {
            let type_id = group.get_type_id();
            // Group 字段是内联的，没有独立的偏移
            (0, Type::StructRef(type_id, Brand { scopes: vec![] }), None)
        }
    };

    Ok(Field {
        name,
        discriminant,
        typ,
        offset,
        default_value,
    })
}

fn extract_default_value(slot: &capnp::schema_capnp::field::slot::Reader) -> Option<DefaultValue> {
    let value_reader = slot.get_default_value().ok()?;
    match value_reader.which() {
        Ok(capnp::schema_capnp::value::Void(())) => None,
        Ok(capnp::schema_capnp::value::Bool(b)) => Some(DefaultValue::Bool(b)),
        Ok(capnp::schema_capnp::value::Int8(v)) => Some(DefaultValue::Int8(v)),
        Ok(capnp::schema_capnp::value::Int16(v)) => Some(DefaultValue::Int16(v)),
        Ok(capnp::schema_capnp::value::Int32(v)) => Some(DefaultValue::Int32(v)),
        Ok(capnp::schema_capnp::value::Int64(v)) => Some(DefaultValue::Int64(v)),
        Ok(capnp::schema_capnp::value::Uint8(v)) => Some(DefaultValue::UInt8(v)),
        Ok(capnp::schema_capnp::value::Uint16(v)) => Some(DefaultValue::UInt16(v)),
        Ok(capnp::schema_capnp::value::Uint32(v)) => Some(DefaultValue::UInt32(v)),
        Ok(capnp::schema_capnp::value::Uint64(v)) => Some(DefaultValue::UInt64(v)),
        Ok(capnp::schema_capnp::value::Float32(v)) => Some(DefaultValue::Float32(v)),
        Ok(capnp::schema_capnp::value::Float64(v)) => Some(DefaultValue::Float64(v)),
        Ok(capnp::schema_capnp::value::Enum(v)) => Some(DefaultValue::Enum(v)),
        // Text, Data, List, Struct, Interface, AnyPointer defaults are pointer-based,
        // handled separately (null pointer = default). Not relevant for XOR mask.
        _ => None,
    }
}

fn parse_enumerant(reader: capnp::schema_capnp::enumerant::Reader) -> Result<Enumerator> {
    let name = reader
        .get_name()
        .with_context(|| "Failed to read enumerant name")?
        .to_string()
        .with_context(|| "Failed to convert enumerant name to string")?;

    let ordinal = reader.get_code_order();

    Ok(Enumerator { name, ordinal })
}

fn parse_type(reader: capnp::schema_capnp::type_::Reader) -> Result<Type> {
    match reader.which()? {
        type_::Void(_) => Ok(Type::Void),
        type_::Bool(_) => Ok(Type::Bool),
        type_::Int8(_) => Ok(Type::Int8),
        type_::Int16(_) => Ok(Type::Int16),
        type_::Int32(_) => Ok(Type::Int32),
        type_::Int64(_) => Ok(Type::Int64),
        type_::Uint8(_) => Ok(Type::UInt8),
        type_::Uint16(_) => Ok(Type::UInt16),
        type_::Uint32(_) => Ok(Type::UInt32),
        type_::Uint64(_) => Ok(Type::UInt64),
        type_::Float32(_) => Ok(Type::Float32),
        type_::Float64(_) => Ok(Type::Float64),
        type_::Text(_) => Ok(Type::Text),
        type_::Data(_) => Ok(Type::Data),
        type_::List(list_type) => {
            let element_type = parse_type(list_type.get_element_type()?)?;
            Ok(Type::List(Box::new(element_type)))
        }
        type_::Struct(struct_type) => {
            // Group 也是结构体引用，只是内存布局不同
            let type_id = struct_type.get_type_id();
            let brand = parse_brand(struct_type.get_brand()?)?;
            Ok(Type::StructRef(type_id, brand))
            // 检查 struct_type 是否有 brand 属性
        }
        type_::Enum(enum_type) => {
            let type_id = enum_type.get_type_id();
            let brand = parse_brand(enum_type.get_brand()?)?;
            Ok(Type::EnumRef(type_id, brand))
        }
        type_::Interface(interface_type) => {
            // 检查 interface_type 是否有 brand 属性
            let type_id = interface_type.get_type_id();
            let brand = parse_brand(interface_type.get_brand()?)?;
            Ok(Type::InterfaceRef(type_id, brand))
        }
        type_::AnyPointer(any_pointer) => {
            match any_pointer.which()? {
                type_::any_pointer::Which::Parameter(param) => {
                    // 这是一个泛型参数
                    Ok(Type::GenericParam(param.get_parameter_index()))
                }
                type_::any_pointer::Which::ImplicitMethodParameter(param) => {
                    // 这是一个方法中的隐式泛型参数
                    Ok(Type::GenericParam(param.get_parameter_index()))
                }
                _ => {
                    // 普通的 AnyPointer 类型
                    Ok(Type::AnyPointer)
                }
            }
        }
    }
}

fn parse_brand(brand_reader: capnp::schema_capnp::brand::Reader) -> Result<Brand> {
    let scopes_reader = brand_reader.get_scopes()?;
    let mut scopes = Vec::new();

    for i in 0..scopes_reader.len() {
        let scope_reader = scopes_reader.get(i);
        let scope = match scope_reader.which()? {
            capnp::schema_capnp::brand::scope::Which::Bind(bind_reader) => {
                let bindings_reader = bind_reader?;
                let mut bindings = Vec::new();

                for j in 0..bindings_reader.len() {
                    let binding_reader = bindings_reader.get(j);
                    let binding = match binding_reader.which()? {
                        capnp::schema_capnp::brand::binding::Which::Unbound(_) => Binding::Unbound,
                        capnp::schema_capnp::brand::binding::Which::Type(type_reader) => {
                            Binding::Type(parse_type(type_reader?)?)
                        }
                    };
                    bindings.push(binding);
                }
                BrandScope::Bind(bindings)
            }
            capnp::schema_capnp::brand::scope::Which::Inherit(_) => BrandScope::Inherit,
        };
        scopes.push(scope);
    }

    Ok(Brand { scopes })
}
