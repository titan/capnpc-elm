use crate::capnproto::{Binding, BrandScope, DefaultValue, Node, NodeKind, RequestedFile, Type};
use crate::elm::{ElmDefaultValue, ElmEnumVariant, ElmField, ElmPrimitiveType, ElmType};
use heck::ToUpperCamelCase;
use std::collections::{HashMap, HashSet};

/// 泛型实例化的单态化任务：持有泛型 struct 节点克隆 + 已映射的类型实参。
/// Elm 的参数化 type alias 在自引用/复合实参下会无限展开，
/// 因此每个 `Result(T)` / `Lookup(T)` 实例化都物化为独立的具体模块。
pub struct SynthJob {
    pub node: Node,
    pub env: Vec<ElmType>,
    pub parent_path: String,
    pub generic_last: String,
    pub mangled: String,
    /// 防碰撞去重键（含实参��名）
    pub dedup_key: String,
}

/// 类型映射上下文：封装 node_map / file_id_to_file / 缓存
pub struct TypeMappingContext<'a> {
    node_map: HashMap<u64, &'a Node>,
    file_id_to_file: HashMap<u64, &'a RequestedFile>,
    cache: HashMap<String, ElmType>,
    /// 泛型参数替换环境（单态化合成期间生效；binding 层设置/清除）
    pub(crate) type_param_env: Option<Vec<ElmType>>,
    /// 待合成的实例化模块
    pending_synths: Vec<SynthJob>,
    synth_seen: HashSet<String>,
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
            type_param_env: None,
            pending_synths: Vec::new(),
            synth_seen: HashSet::new(),
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

    /// 将 Cap'n Proto 类型映射到 Elm 类型（含缓存；替换环境下旁路缓存）
    pub fn map_type(&mut self, capnp_type: &Type, current_node_id: u64) -> ElmType {
        if self.type_param_env.is_some() {
            return self.map_type_inner(capnp_type, current_node_id);
        }

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

                // 泛型 struct 的已绑定实例化 → 单态化为具体模块
                let is_generic_def = self
                    .node_map
                    .get(id)
                    .is_some_and(|n| !n.generic_params.is_empty());
                if is_generic_def && !type_args.is_empty() {
                    return self.enqueue_generic_instantiation(*id, type_args);
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
                // 单态化环境下直接替换为具体实参
                if let Some(env) = &self.type_param_env {
                    if let Some(t) = env.get(*index as usize) {
                        return t.clone();
                    }
                }
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

    /// 登记一个泛型实例化的单态化任务，返回指向合成模块的 Elm 引用。
    fn enqueue_generic_instantiation(&mut self, id: u64, type_args: Vec<ElmType>) -> ElmType {
        let mangled = format!(
            "Of{}",
            type_args.iter().map(Self::mangle_arg).collect::<Vec<_>>().join("And")
        );
        let parent_full = self.get_full_type_name(id);
        let generic_last = parent_full
            .rsplit('.')
            .next()
            .unwrap_or(&parent_full)
            .to_string();
        let parent_path = parent_full
            .strip_suffix(&format!(".{generic_last}"))
            .unwrap_or("")
            .to_string();
        let synth_full = if parent_path.is_empty() {
            format!("{generic_last}.{mangled}")
        } else {
            format!("{parent_path}.{generic_last}.{mangled}")
        };
        // 去重键包含实参全名，避免不同实参意外并成同一模块
        let dedup_key = format!(
            "{}|{}",
            synth_full,
            type_args
                .iter()
                .map(|a| a.to_elm_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        if self.synth_seen.insert(dedup_key.clone()) {
            if let Some(node) = self.node_map.get(&id) {
                let mut node = (*node).clone();
                // 克隆节点不再是泛型定义：模板按具体模块渲染
                node.generic_params = Vec::new();
                self.pending_synths.push(SynthJob {
                    node,
                    env: type_args,
                    parent_path,
                    generic_last,
                    mangled,
                    dedup_key,
                });
            }
        }
        ElmType::StructRef(synth_full, "Entity", vec![])
    }

    /// ElmType → 模块名段（单态化命名用）
    fn mangle_arg(t: &ElmType) -> String {
        match t {
            ElmType::Primitive(p) => match p {
                ElmPrimitiveType::Bool => "Bool".to_string(),
                ElmPrimitiveType::Int(64) => "Word64".to_string(),
                ElmPrimitiveType::Int(_) => "Int".to_string(),
                ElmPrimitiveType::Float(_) => "Float".to_string(),
                ElmPrimitiveType::String => "String".to_string(),
                ElmPrimitiveType::Bytes => "Bytes".to_string(),
                ElmPrimitiveType::Unit => "Void".to_string(),
            },
            ElmType::AnyPointer => "AnyPointer".to_string(),
            ElmType::InterfaceRef(m, _, _) => m
                .rsplit('.')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("Cap")
                .to_string(),
            ElmType::List(inner) => format!("ListOf{}", Self::mangle_arg(inner)),
            ElmType::StructRef(m, _, _) | ElmType::EnumRef(m, _, _, _) => m
                .rsplit('.')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("Anon")
                .to_string(),
            ElmType::UnionInline(_, _) => "Union".to_string(),
            ElmType::GenericParam(n) => n.to_upper_camel_case(),
        }
    }

    pub fn pop_pending_synth(&mut self) -> Option<SynthJob> {
        self.pending_synths.pop()
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
    /// Works for method param structs as well as result structs.
    pub fn struct_type_has_interface_fields(&self, capnp_type: &Type) -> bool {
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
                let mut key = format!("StructRef:{}@{}", id, current_node_id);
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
                let mut key = format!("EnumRef:{}@{}", id, current_node_id);
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
