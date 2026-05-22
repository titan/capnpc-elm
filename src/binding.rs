use crate::capnproto::{
    Binding, BrandScope, DefaultValue, Enumerator, Field, Node, NodeKind, RequestedFile, Type,
};
use crate::elm::{
    ElmContext, ElmDefaultValue, ElmEnumVariant, ElmField, ElmMethod, ElmModule, ElmPrimitiveType,
    ElmType, ElmTypeDef, ElmUnionBranch,
};
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::collections::HashMap;
use std::sync::Mutex;

lazy_static::lazy_static! {
    static ref TYPE_CACHE: Mutex<HashMap<String, ElmType>> = Mutex::new(HashMap::new());
}

/// 将 Cap'n Proto DefaultValue 映射到 Elm 默认值
fn map_default_value(default: &Option<DefaultValue>) -> Option<ElmDefaultValue> {
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

/// 将 Cap'n Proto 节点转换为 Elm 上下文
pub fn generate_elm_context(nodes: &[Node], requested_files: &[RequestedFile]) -> ElmContext {
    // 构建文件ID到文件的映射
    let file_id_to_file: HashMap<u64, &RequestedFile> =
        requested_files.iter().map(|file| (file.id, file)).collect();

    // 构建节点ID到节点的映射表
    let mut node_map = HashMap::new();
    build_node_ref_map(nodes, &mut node_map);

    let mut context = ElmContext::new();

    // 处理每个节点
    for node in nodes {
        convert_node_to_elm(node, &mut context, &node_map, &file_id_to_file);
    }

    context
}

/// Append RPC type modules parsed from rpc.capnp into the existing context.
///
/// The `rpc_files` filename is rewritten to `rpc.capnp` so that `get_full_type_name`
/// produces `Rpc.X` module paths (not the full system path).
pub fn append_rpc_modules(
    context: &mut ElmContext,
    rpc_nodes: &[Node],
    rpc_files: &[RequestedFile],
) {
    // Rewrite filenames to plain "rpc.capnp" → generates "Rpc" prefix
    let rewritten_files: Vec<RequestedFile> = rpc_files
        .iter()
        .map(|f| RequestedFile {
            id: f.id,
            filename: "rpc.capnp".to_string(),
        })
        .collect();

    let file_id_to_file: HashMap<u64, &RequestedFile> =
        rewritten_files.iter().map(|file| (file.id, file)).collect();

    let mut node_map = HashMap::new();
    build_node_ref_map(rpc_nodes, &mut node_map);

    for node in rpc_nodes {
        convert_node_to_elm(node, context, &node_map, &file_id_to_file);
    }
}

/// 递归构建节点ID映射表
fn build_node_ref_map<'a>(nodes: &'a [Node], node_map: &mut HashMap<u64, &'a Node>) {
    for node in nodes {
        node_map.insert(node.id, node);
        build_node_ref_map(&node.nested_nodes, node_map);
    }
}

/// 将 Cap'n Proto 节点转换为 Elm 模块
fn convert_node_to_elm(
    node: &Node,
    context: &mut ElmContext,
    node_map: &HashMap<u64, &Node>,
    file_id_to_file: &HashMap<u64, &RequestedFile>,
) {
    // 直接获取完整模块路径
    let full_module_name = get_full_type_name(node.id, node_map, file_id_to_file);

    // 提取纯类型名（最后一部分）
    let type_name = extract_type_name(&node.display_name);

    // 提取父模块路径（用于组织模块层次）
    let module_path = if let Some(last_dot) = full_module_name.rfind('.') {
        full_module_name[..last_dot].to_string()
    } else {
        "".to_string()
    };

    // 泛型参数
    let generic_params = node
        .generic_params
        .iter()
        .map(|x| x.to_lowercase())
        .collect();

    if let NodeKind::Interface(methods) = &node.kind {
        let mut imports = vec![];
        let mut elm_methods = Vec::with_capacity(methods.len());

        for method in methods {
            let param_type =
                map_capnp_type_to_elm(&method.param_type, node_map, file_id_to_file, node.id);
            let result_type =
                map_capnp_type_to_elm(&method.result_type, node_map, file_id_to_file, node.id);

            collect_imports_from_type(&param_type, &mut imports);
            collect_imports_from_type(&result_type, &mut imports);
            let param_has_caps = param_type.contains_interface_ref();
            // result_type 是 StructRef 时，contains_interface_ref() 只检查泛型参数
            // 但结果 struct 的字段（如 EchoFactory.create → CreateResults.echo）可能含 interface
            // 需要额外查看对应 struct node 的 fields
            let result_has_caps = result_type.contains_interface_ref()
                || result_struct_has_interface_fields(&method.result_type, node_map);

            elm_methods.push(ElmMethod {
                id: method.id,
                name: method.name.clone(),
                implicit_parameters: method.implicit_parameters.clone(),
                param_type,
                result_type,
                param_has_caps,
                result_has_caps,
            });
        }
        imports.push("Capnproto".to_owned());
        imports.push("Rpc.Client as Rpc".to_owned());

        // 去重导入
        imports.sort();
        imports.dedup();

        let module = ElmModule {
            id: node.id,
            name: type_name,
            path: module_path,
            imports,
            type_def: ElmTypeDef::Interface,
            data_words: 0,
            pointer_words: 0,
            fields: vec![],
            discriminant_offset: 0,
            methods: elm_methods,
            generic_params,
        };

        context.modules.push(module);

        // 接口节点不创建 ElmModule，直接递归处理嵌套节点后返回
        for nested in &node.nested_nodes {
            convert_node_to_elm(nested, context, node_map, file_id_to_file);
        }
        return;
    }

    let mut imports = vec![];
    let mut fields = Vec::new();
    let mut data_words: u32 = 0;
    let mut pointer_words: u32 = 0;
    let mut discriminant_offset = 0;

    let type_def = match &node.kind {
        NodeKind::Struct {
            is_group,
            fields: capnp_fields,
            data_word_count,
            pointer_word_count,
            union_fields,
            discriminant_offset: offset,
        } => {
            eprintln!("DEBUG: node={} kind={:?}", node.display_name, node.kind);
            data_words = *data_word_count as u32;
            pointer_words = *pointer_word_count as u32;
            discriminant_offset = *offset;

            convert_fields(
                capnp_fields,
                &mut fields,
                &mut imports,
                node_map,
                file_id_to_file,
                node.id,
            );

            if let Some(union_fields) = union_fields {
                let mut branches = Vec::new();
                for field in union_fields {
                    let branch = ElmUnionBranch {
                        name: field.name.clone(),
                        discriminant: field.discriminant.unwrap(),
                        elm_type: map_capnp_type_to_elm(
                            &field.typ,
                            node_map,
                            file_id_to_file,
                            node.id,
                        ),
                        offset: field.offset,
                        is_pointer: is_pointer_type(&field.typ),
                        default_value: map_default_value(&field.default_value),
                    };
                    branches.push(branch);
                }

                let unnamed_union = ElmField {
                    name: "unnamedUnion".to_string(), // 固定字段名
                    discriminant: None,
                    elm_type: ElmType::UnionInline(branches, generic_params.clone()),
                    offset: discriminant_offset,
                    is_union_container: true,
                    default_value: None,
                };

                collect_imports_from_type(&unnamed_union.elm_type, &mut imports);

                fields.push(unnamed_union);
            } else {
                // 处理内联的联合体节点（有命名的内嵌union）
                eprintln!(
                    "DEBUG: checking nested nodes for {} ({} nested)",
                    node.display_name,
                    node.nested_nodes.len()
                );
                for nested in &node.nested_nodes {
                    if let NodeKind::Struct {
                        is_group: true,
                        union_fields: Some(capnp_fields),
                        discriminant_offset: offset,
                        ..
                    } = &nested.kind
                    {
                        discriminant_offset = *offset;
                        let mut branches = Vec::new();
                        for field in capnp_fields {
                            let branch = ElmUnionBranch {
                                name: field.name.clone(),
                                discriminant: field.discriminant.unwrap_or(0),
                                elm_type: map_capnp_type_to_elm(
                                    &field.typ,
                                    node_map,
                                    file_id_to_file,
                                    node.id,
                                ),
                                offset: field.offset,
                                is_pointer: is_pointer_type(&field.typ),
                                default_value: map_default_value(&field.default_value),
                            };
                            branches.push(branch);
                        }

                        let named_union = ElmField {
                            name: extract_type_name(&nested.display_name).to_lower_camel_case(),
                            discriminant: None,
                            elm_type: ElmType::UnionInline(branches, generic_params.clone()),
                            offset: *offset,
                            is_union_container: true,
                            default_value: None,
                        };

                        collect_imports_from_type(&named_union.elm_type, &mut imports);
                        eprintln!(
                            "DEBUG named_union for {}: branches={:?}",
                            extract_type_name(&nested.display_name),
                            named_union.elm_type
                        );

                        fields.push(named_union);
                    }
                }
            }

            ElmTypeDef::Struct
        }
        NodeKind::Enum(variants) => {
            convert_enum_to_fields(node.id, variants, &mut fields, node_map, file_id_to_file);
            ElmTypeDef::Enum
        }
        _ => {
            imports.push("Capnproto".to_owned());
            ElmTypeDef::Struct
        }
    };
    // All struct/enum modules use Capnproto types (StructLayout, Reader, AnyPointer, etc.)
    // in encode, decode, toAnyPointer, fromAnyPointer - always import it
    if !matches!(type_def, ElmTypeDef::Interface) {
        imports.push("Capnproto".to_owned());
    }

    // Add Bitwise import if any field has non-zero defaults requiring XOR
    collect_imports_from_defaults(&fields, &mut imports);

    // 去重导入
    imports.sort();
    imports.dedup();

    imports.retain(|import| import != &full_module_name);

    let module = ElmModule {
        id: node.id,
        name: type_name,
        path: module_path,
        imports,
        type_def,
        data_words,
        pointer_words,
        fields,
        discriminant_offset,
        methods: vec![],
        generic_params,
    };

    context.modules.push(module);

    // 递归处理嵌套节点
    for nested in &node.nested_nodes {
        convert_node_to_elm(nested, context, node_map, file_id_to_file);
    }
}

fn convert_fields(
    capnp_fields: &[Field],
    fields: &mut Vec<ElmField>,
    imports: &mut Vec<String>,
    node_map: &HashMap<u64, &Node>,
    file_id_to_file: &HashMap<u64, &RequestedFile>,
    current_node_id: u64,
) {
    for field in capnp_fields {
        if let Type::StructRef(id, _) = field.typ {
            if let Some(node) = node_map.get(&id) {
                if let NodeKind::Struct { is_group: true, .. } = &node.kind {
                    continue; // 跳过 group 字段
                }
            }
        }

        let elm_type =
            map_capnp_type_to_elm(&field.typ, node_map, file_id_to_file, current_node_id);
        collect_imports_from_type(&elm_type, imports);
        let is_union_container = matches!(elm_type, ElmType::UnionInline(..));

        fields.push(ElmField {
            name: field.name.clone(),
            discriminant: field.discriminant,
            elm_type,
            offset: field.offset,
            is_union_container,
            default_value: map_default_value(&field.default_value),
        });
    }
}

/// 转换枚举为字段
fn convert_enum_to_fields(
    node_id: u64,
    variants: &[Enumerator],
    fields: &mut Vec<ElmField>,
    node_map: &HashMap<u64, &Node>,
    file_id_to_file: &HashMap<u64, &RequestedFile>,
) {
    // 枚举作为一个整体字段，包含所有变体信息
    let mut enum_branches = Vec::new();

    for variant in variants {
        enum_branches.push(ElmEnumVariant {
            name: variant.name.clone(),
            ordinal: variant.ordinal,
        });
    }

    let full_name = get_full_type_name(node_id, node_map, file_id_to_file);

    fields.push(ElmField {
        name: "ignored".to_string(),
        discriminant: None,
        elm_type: ElmType::EnumRef(full_name, "Entity", enum_branches, vec![]),
        offset: 0,
        is_union_container: false,
        default_value: None,
    });
}

/// 将 Cap'n Proto 类型映射到 Elm 类型
fn map_capnp_type_to_elm(
    capnp_type: &Type,
    node_map: &HashMap<u64, &Node>,
    file_id_to_file: &HashMap<u64, &RequestedFile>,
    current_node_id: u64,
) -> ElmType {
    let cache_key = type_to_cache_key(capnp_type, node_map, file_id_to_file, current_node_id);

    if let Some(cached) = TYPE_CACHE.lock().unwrap().get(&cache_key) {
        return cached.clone();
    }

    let result = match capnp_type {
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
            let inner_type =
                map_capnp_type_to_elm(inner, node_map, file_id_to_file, current_node_id);
            ElmType::List(Box::new(inner_type))
        }
        Type::StructRef(id, brand) => {
            let module_name = if *id == current_node_id {
                String::new()
            } else {
                get_full_type_name(*id, node_map, file_id_to_file)
            };

            // 处理 Brand 信息
            let mut type_args = Vec::new();
            for scope in &brand.scopes {
                if let BrandScope::Bind(bindings) = scope {
                    for binding in bindings {
                        if let Binding::Type(t) = binding {
                            type_args.push(map_capnp_type_to_elm(
                                t,
                                node_map,
                                file_id_to_file,
                                current_node_id,
                            ));
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
                get_full_type_name(*id, node_map, file_id_to_file)
            };

            // 处理 Brand 信息
            let mut type_args = Vec::new();
            for scope in &brand.scopes {
                if let BrandScope::Bind(bindings) = scope {
                    for binding in bindings {
                        if let Binding::Type(t) = binding {
                            type_args.push(map_capnp_type_to_elm(
                                t,
                                node_map,
                                file_id_to_file,
                                current_node_id,
                            ));
                        }
                    }
                }
            }
            if let Some(node) = node_map.get(id) {
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
                get_full_type_name(*id, node_map, file_id_to_file)
            };

            // 处理 Brand 信息
            let mut type_args = Vec::new();
            for scope in &brand.scopes {
                if let BrandScope::Bind(bindings) = scope {
                    for binding in bindings {
                        if let Binding::Type(t) = binding {
                            type_args.push(map_capnp_type_to_elm(
                                t,
                                node_map,
                                file_id_to_file,
                                current_node_id,
                            ));
                        }
                    }
                }
            }
            ElmType::InterfaceRef(module_name, "Entity", type_args)
        }
        Type::AnyPointer => ElmType::AnyPointer,
        Type::Void => ElmType::Primitive(ElmPrimitiveType::Unit),
        Type::GenericParam(index) => {
            // 查找当前节点
            if let Some(node) = node_map.get(&current_node_id) {
                // 从节点的泛型参数列表中获取实际参数名
                if (*index as usize) < node.generic_params.len() {
                    ElmType::GenericParam(node.generic_params[*index as usize].to_lowercase())
                } else {
                    // 如果索引超出范围，回退到使用 "t"
                    ElmType::GenericParam("t".to_string())
                }
            } else {
                // 如果找不到当前节点，回退到使用 "t"
                ElmType::GenericParam("t".to_string())
            }
        }
    };

    // 存入缓存
    TYPE_CACHE.lock().unwrap().insert(cache_key, result.clone());

    result
}

/// 检查类型是否是指针类型
fn is_pointer_type(typ: &Type) -> bool {
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
fn collect_imports_from_type(elm_type: &ElmType, imports: &mut Vec<String>) {
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
                collect_imports_from_type(brand, imports);
            }
        }
        ElmType::EnumRef(module_name, _, _, brands) => {
            if !module_name.is_empty() {
                imports.push(module_name.clone());
            }
            for brand in brands {
                collect_imports_from_type(brand, imports);
            }
        }
        ElmType::InterfaceRef(_, _, brands) => {
            imports.push("Rpc.Client as Rpc".to_owned());
            for brand in brands {
                collect_imports_from_type(brand, imports);
            }
        }
        ElmType::List(inner) => {
            collect_imports_from_type(inner, imports);
        }
        ElmType::UnionInline(branches, _) => {
            for branch in branches {
                collect_imports_from_type(&branch.elm_type, imports);
                // Union branches may have default values requiring Bitwise.xor
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
fn collect_imports_from_defaults(fields: &[ElmField], imports: &mut Vec<String>) {
    for field in fields {
        if field
            .default_value
            .as_ref()
            .map_or(false, |dv| !dv.is_zero())
        {
            imports.push("Bitwise".to_owned());
        }
        // 也检查 union branches
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

/// 从 display_name 中提取纯类型名
fn extract_type_name(display_name: &str) -> String {
    display_name
        .split(':')
        .next_back()
        .unwrap_or(display_name)
        .split('.')
        .next_back()
        .unwrap_or(display_name)
        .to_upper_camel_case()
}

/// 获取节点的完整类型名称
fn get_full_type_name(
    node_id: u64,
    node_map: &HashMap<u64, &Node>,
    file_id_to_file: &HashMap<u64, &RequestedFile>,
) -> String {
    if let Some(node) = node_map.get(&node_id) {
        if node.display_name.contains("Message") && node.display_name.contains("rpc") {
            eprintln!(
                "DEBUG get_full_type_name: display_name={}, file_id={}, file={:?}",
                node.display_name,
                node.file_id,
                file_id_to_file.get(&node.file_id).map(|f| &f.filename)
            );
        }
        // 从 display_name 提取完整路径并规范化
        let full_path = node
            .display_name
            .split(':')
            .next_back()
            .unwrap_or(&node.display_name)
            .split('.')
            .map(|s| s.to_upper_camel_case())
            .collect::<Vec<_>>()
            .join(".");

        // 获取节点所属文件
        let file_path = if let Some(file) = file_id_to_file.get(&node.file_id) {
            file.filename
                .replace(".capnp", "")
                .split('/')
                .map(|s| s.to_upper_camel_case())
                .collect::<Vec<_>>()
                .join(".")
        } else {
            String::new()
        };

        // 组合文件路径和节点路径
        let combined = if file_path.is_empty() {
            full_path
        } else {
            format!("{}.{}", file_path, full_path)
        };

        // Capnp.Rpc.* → Rpc.*: RPC modules live directly under the Rpc package
        combined.replace("Capnp.Rpc.", "Rpc.")
    } else {
        format!("Unknown{}", node_id)
    }
}

/// 生成类型缓存键，包含类型信息和品牌信息
fn type_to_cache_key(
    capnp_type: &Type,
    node_map: &HashMap<u64, &Node>,
    file_id_to_file: &HashMap<u64, &RequestedFile>,
    current_node_id: u64,
) -> String {
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
                                    type_to_cache_key(
                                        t,
                                        node_map,
                                        file_id_to_file,
                                        current_node_id
                                    )
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
                                    type_to_cache_key(
                                        t,
                                        node_map,
                                        file_id_to_file,
                                        current_node_id
                                    )
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
                                    type_to_cache_key(
                                        t,
                                        node_map,
                                        file_id_to_file,
                                        current_node_id
                                    )
                                ));
                            }
                        }
                    }
                }
            }
            key
        }
        Type::List(inner) => {
            format!(
                "List:{}",
                type_to_cache_key(inner, node_map, file_id_to_file, current_node_id)
            )
        }
        Type::GenericParam(index) => {
            if let Some(node) = node_map.get(&current_node_id) {
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

/// Check if a struct type's fields contain any InterfaceRef (capability).
/// Used to correctly compute `result_has_caps` for interface methods where
/// the result struct has capability fields (e.g., `create @0 () -> (cap : Interface)`).
fn result_struct_has_interface_fields(capnp_type: &Type, node_map: &HashMap<u64, &Node>) -> bool {
    // Get the struct node ID
    let node_id = match capnp_type {
        Type::StructRef(id, _) => *id,
        _ => return false,
    };

    // Look up the node
    let node = match node_map.get(&node_id) {
        Some(node) => node,
        None => return false,
    };

    // Check struct fields for InterfaceRef
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
