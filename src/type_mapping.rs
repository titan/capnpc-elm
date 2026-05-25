use crate::capnproto::{Binding, BrandScope, DefaultValue, Node, NodeKind, RequestedFile, Type};
use crate::elm::{ElmDefaultValue, ElmEnumVariant, ElmField, ElmPrimitiveType, ElmType};
use heck::ToUpperCamelCase;
use std::collections::HashMap;

/// 类型映射上下文：封装 node_map / file_id_to_file / 缓存
pub struct TypeMappingContext<'a> {
    node_map: HashMap<u64, &'a Node>,
    file_id_to_file: HashMap<u64, &'a RequestedFile>,
    cache: HashMap<String, ElmType>,
}

impl<'a> TypeMappingContext<'a> {
    /// 从节点列表和请求文件列表构建映射上下文
    pub fn new(nodes: &'a [Node], file_id_to_file: HashMap<u64, &'a RequestedFile>) -> Self {
        let mut node_map = HashMap::new();
        Self::build_node_ref_map(nodes, &mut node_map);
        TypeMappingContext {
            node_map,
            file_id_to_file,
            cache: HashMap::new(),
        }
    }

    /// 递归构建节点ID映射表
    fn build_node_ref_map(nodes: &'a [Node], node_map: &mut HashMap<u64, &'a Node>) {
        for node in nodes {
            node_map.insert(node.id, node);
            Self::build_node_ref_map(&node.nested_nodes, node_map);
        }
    }

    /// 检查指定节点ID是否为 group struct
    pub fn is_group_struct(&self, node_id: u64) -> bool {
        self.node_map
            .get(&node_id)
            .is_some_and(|node| matches!(node.kind, NodeKind::Struct { is_group: true, .. }))
    }

    /// 将 Cap'n Proto 类型映射到 Elm 类型（含缓存）
    pub fn map_type(&mut self, capnp_type: &Type, current_node_id: u64) -> ElmType {
        let cache_key = self.type_to_cache_key(capnp_type, current_node_id);

        if let Some(cached) = self.cache.get(&cache_key) {
            return cached.clone();
        }

        let result = self.map_type_inner(capnp_type, current_node_id);

        self.cache.insert(cache_key, result.clone());

        result
    }

    fn map_type_inner(&mut self, capnp_type: &Type, current_node_id: u64) -> ElmType {
        match capnp_type {
            Type::Bool => ElmType::Primitive(ElmPrimitiveType::Bool),
            Type::Int8 => ElmType::Primitive(ElmPrimitiveType::Int(8)),
            Type::Int16 => ElmType::Primitive(ElmPrimitiveType::Int(16)),
            Type::Int32 => ElmType::Primitive(ElmPrimitiveType::Int(32)),
            Type::Int64 => ElmType::Primitive(ElmPrimitiveType::Int(64)),
            Type::UInt8 => ElmType::Primitive(ElmPrimitiveType::Int(8)),
            Type::UInt16 => ElmType::Primitive(ElmPrimitiveType::Int(16)),
            Type::UInt32 => ElmType::Primitive(ElmPrimitiveType::Int(32)),
            Type::UInt64 => ElmType::Primitive(ElmPrimitiveType::Int(64)),
            Type::Float32 => ElmType::Primitive(ElmPrimitiveType::Float(32)),
            Type::Float64 => ElmType::Primitive(ElmPrimitiveType::Float(64)),
            Type::Text => ElmType::Primitive(ElmPrimitiveType::String),
            Type::Data => ElmType::Primitive(ElmPrimitiveType::Bytes),
            Type::List(inner) => {
                let inner_type = self.map_type(inner, current_node_id);
                ElmType::List(Box::new(inner_type))
            }
            Type::StructRef(id, brand) => {
                let module_name = if *id == current_node_id {
                    String::new()
                } else {
                    self.get_full_type_name(*id)
                };

                let mut type_args = Vec::new();
                for scope in &brand.scopes {
                    if let BrandScope::Bind(bindings) = scope {
                        for binding in bindings {
                            if let Binding::Type(t) = binding {
                                type_args.push(self.map_type(t, current_node_id));
                            }
                        }
                    }
                }

                ElmType::StructRef(module_name, "Entity", type_args)
            }
            Type::EnumRef(id, brand) => {
                let module_name = if *id == current_node_id {
                    String::new()
                } else {
                    self.get_full_type_name(*id)
                };

                let mut type_args = Vec::new();
                for scope in &brand.scopes {
                    if let BrandScope::Bind(bindings) = scope {
                        for binding in bindings {
                            if let Binding::Type(t) = binding {
                                type_args.push(self.map_type(t, current_node_id));
                            }
                        }
                    }
                }
                if let Some(node) = self.node_map.get(id) {
                    if let NodeKind::Enum(capnp_variants) = &node.kind {
                        let variants = capnp_variants
                            .iter()
                            .map(|x| ElmEnumVariant {
                                name: x.name.clone(),
                                ordinal: x.ordinal,
                            })
                            .collect();
                        return ElmType::EnumRef(module_name, "Entity", variants, type_args);
                    }
                }
                ElmType::EnumRef(module_name, "Entity", vec![], type_args)
            }
            Type::InterfaceRef(id, brand) => {
                let module_name = if *id == current_node_id {
                    String::new()
                } else {
                    self.get_full_type_name(*id)
                };

                let mut type_args = Vec::new();
                for scope in &brand.scopes {
                    if let BrandScope::Bind(bindings) = scope {
                        for binding in bindings {
                            if let Binding::Type(t) = binding {
                                type_args.push(self.map_type(t, current_node_id));
                            }
                        }
                    }
                }
                ElmType::InterfaceRef(module_name, "Entity", type_args)
            }
            Type::AnyPointer => ElmType::AnyPointer,
            Type::Void => ElmType::Primitive(ElmPrimitiveType::Unit),
            Type::GenericParam(index) => {
                if let Some(node) = self.node_map.get(&current_node_id) {
                    if (*index as usize) < node.generic_params.len() {
                        ElmType::GenericParam(node.generic_params[*index as usize].to_lowercase())
                    } else {
                        ElmType::GenericParam("t".to_string())
                    }
                } else {
                    ElmType::GenericParam("t".to_string())
                }
            }
        }
    }

    /// 获取节点的完整类型名称
    pub fn get_full_type_name(&self, node_id: u64) -> String {
        if let Some(node) = self.node_map.get(&node_id) {
            let full_path = node
                .display_name
                .split(':')
                .next_back()
                .unwrap_or(&node.display_name)
                .split('.')
                .map(|s| s.to_upper_camel_case())
                .collect::<Vec<_>>()
                .join(".");

            let file_path = if let Some(file) = self.file_id_to_file.get(&node.file_id) {
                file.filename
                    .replace(".capnp", "")
                    .split('/')
                    .map(|s| s.to_upper_camel_case())
                    .collect::<Vec<_>>()
                    .join(".")
            } else {
                String::new()
            };

            let combined = if file_path.is_empty() {
                full_path
            } else {
                format!("{}.{}", file_path, full_path)
            };

            combined.replace("Capnp.Rpc.", "Rpc.")
        } else {
            format!("Unknown{}", node_id)
        }
    }

    /// 从 display_name 中提取纯类型名
    pub fn extract_type_name(display_name: &str) -> String {
        display_name
            .split(':')
            .next_back()
            .unwrap_or(display_name)
            .split('.')
            .next_back()
            .unwrap_or(display_name)
            .to_upper_camel_case()
    }

    /// 检查类型是否是指针类型
    pub fn is_pointer_type(typ: &Type) -> bool {
        matches!(
            typ,
            Type::Text
                | Type::Data
                | Type::List(_)
                | Type::StructRef(_, _)
                | Type::InterfaceRef(_, _)
                | Type::GenericParam(_)
                | Type::AnyPointer
        )
    }

    /// 从类型中收集导入
    pub fn collect_imports_from_type(elm_type: &ElmType, imports: &mut Vec<String>) {
        match elm_type {
            ElmType::Primitive(ElmPrimitiveType::Int(64)) => {
                imports.push("Capnproto".to_owned());
            }
            ElmType::Primitive(ElmPrimitiveType::Bytes) => {
                imports.push("Bytes".to_owned());
                imports.push("Capnproto".to_owned());
            }
            ElmType::StructRef(module_name, _, brands) => {
                if !module_name.is_empty() {
                    imports.push(module_name.clone());
                }
                for brand in brands {
                    Self::collect_imports_from_type(brand, imports);
                }
            }
            ElmType::EnumRef(module_name, _, _, brands) => {
                if !module_name.is_empty() {
                    imports.push(module_name.clone());
                }
                for brand in brands {
                    Self::collect_imports_from_type(brand, imports);
                }
            }
            ElmType::InterfaceRef(_, _, brands) => {
                imports.push("Rpc.Client as Rpc".to_owned());
                for brand in brands {
                    Self::collect_imports_from_type(brand, imports);
                }
            }
            ElmType::List(inner) => {
                Self::collect_imports_from_type(inner, imports);
            }
            ElmType::UnionInline(branches, _) => {
                for branch in branches {
                    Self::collect_imports_from_type(&branch.elm_type, imports);
                    if branch
                        .default_value
                        .as_ref()
                        .map_or(false, |dv| !dv.is_zero())
                    {
                        imports.push("Bitwise".to_owned());
                    }
                }
            }
            _ => {}
        }
    }

    /// 从字段列表中收集因默认值 XOR 需要的导入（如 Bitwise）
    pub fn collect_imports_from_defaults(fields: &[ElmField], imports: &mut Vec<String>) {
        for field in fields {
            if field
                .default_value
                .as_ref()
                .map_or(false, |dv| !dv.is_zero())
            {
                imports.push("Bitwise".to_owned());
            }
            if let ElmType::UnionInline(branches, _) = &field.elm_type {
                for branch in branches {
                    if branch
                        .default_value
                        .as_ref()
                        .map_or(false, |dv| !dv.is_zero())
                    {
                        imports.push("Bitwise".to_owned());
                    }
                }
            }
        }
    }

    /// 将 Cap'n Proto DefaultValue 映射到 Elm 默认值
    pub fn map_default_value(default: &Option<DefaultValue>) -> Option<ElmDefaultValue> {
        default.as_ref().map(|dv| match dv {
            DefaultValue::Bool(b) => ElmDefaultValue::Bool(*b),
            DefaultValue::Int8(v) => ElmDefaultValue::Int(*v as i64),
            DefaultValue::Int16(v) => ElmDefaultValue::Int(*v as i64),
            DefaultValue::Int32(v) => ElmDefaultValue::Int(*v as i64),
            DefaultValue::Int64(v) => ElmDefaultValue::Int(*v),
            DefaultValue::UInt8(v) => ElmDefaultValue::Int(*v as i64),
            DefaultValue::UInt16(v) => ElmDefaultValue::Int(*v as i64),
            DefaultValue::UInt32(v) => ElmDefaultValue::Int(*v as i64),
            DefaultValue::UInt64(v) => ElmDefaultValue::Int(*v as i64),
            DefaultValue::Float32(v) => ElmDefaultValue::Float(*v as f64),
            DefaultValue::Float64(v) => ElmDefaultValue::Float(*v),
            DefaultValue::Enum(v) => ElmDefaultValue::Enum(*v),
        })
    }

    /// Check if a struct type's fields contain any InterfaceRef (capability).
    pub fn result_struct_has_interface_fields(&self, capnp_type: &Type) -> bool {
        let node_id = match capnp_type {
            Type::StructRef(id, _) => *id,
            _ => return false,
        };

        let node = match self.node_map.get(&node_id) {
            Some(node) => node,
            None => return false,
        };

        match &node.kind {
            NodeKind::Struct {
                fields,
                union_fields,
                ..
            } => {
                let has_interface = fields
                    .iter()
                    .any(|f| matches!(f.typ, Type::InterfaceRef(_, _)));
                let has_union_interface = union_fields
                    .as_ref()
                    .is_some_and(|u| u.iter().any(|f| matches!(f.typ, Type::InterfaceRef(_, _))));
                has_interface || has_union_interface
            }
            _ => false,
        }
    }

    /// 生成类型缓存键，包含类型信息和品牌信息
    fn type_to_cache_key(&self, capnp_type: &Type, current_node_id: u64) -> String {
        match capnp_type {
            Type::StructRef(id, brand) => {
                let mut key = format!("StructRef:{}", id);
                for scope in &brand.scopes {
                    if let BrandScope::Bind(bindings) = scope {
                        for binding in bindings {
                            match binding {
                                Binding::Unbound => key.push_str(":Unbound"),
                                Binding::Type(t) => {
                                    key.push_str(&format!(
                                        ":Type:{}",
                                        self.type_to_cache_key(t, current_node_id)
                                    ));
                                }
                            }
                        }
                    }
                }
                key
            }
            Type::EnumRef(id, brand) => {
                let mut key = format!("EnumRef:{}", id);
                for scope in &brand.scopes {
                    if let BrandScope::Bind(bindings) = scope {
                        for binding in bindings {
                            match binding {
                                Binding::Unbound => key.push_str(":Unbound"),
                                Binding::Type(t) => {
                                    key.push_str(&format!(
                                        ":Type:{}",
                                        self.type_to_cache_key(t, current_node_id)
                                    ));
                                }
                            }
                        }
                    }
                }
                key
            }
            Type::InterfaceRef(id, brand) => {
                let mut key = format!("InterfaceRef:{}", id);
                for scope in &brand.scopes {
                    if let BrandScope::Bind(bindings) = scope {
                        for binding in bindings {
                            match binding {
                                Binding::Unbound => key.push_str(":Unbound"),
                                Binding::Type(t) => {
                                    key.push_str(&format!(
                                        ":Type:{}",
                                        self.type_to_cache_key(t, current_node_id)
                                    ));
                                }
                            }
                        }
                    }
                }
                key
            }
            Type::List(inner) => {
                format!("List:{}", self.type_to_cache_key(inner, current_node_id))
            }
            Type::GenericParam(index) => {
                if let Some(node) = self.node_map.get(&current_node_id) {
                    if (*index as usize) < node.generic_params.len() {
                        format!("GenericParam:{}", node.generic_params[*index as usize])
                    } else {
                        format!("GenericParam:{}", index)
                    }
                } else {
                    format!("GenericParam:{}", index)
                }
            }
            _ => format!("{:?}", capnp_type),
        }
    }
}
