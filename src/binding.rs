use crate::capnproto::{Field, Node, NodeKind, RequestedFile, Type};
use crate::elm::{ElmContext, ElmField, ElmMethod, ElmModule, ElmType, ElmTypeDef, ElmUnionBranch};
use crate::type_mapping::TypeMappingContext;
use heck::ToLowerCamelCase;
use std::collections::HashMap;

/// 将 Cap'n Proto 节点转换为 Elm 上下文
pub fn generate_elm_context(nodes: &[Node], requested_files: &[RequestedFile]) -> ElmContext {
    // 构建文件ID到文件的映射
    let file_id_to_file: HashMap<u64, &RequestedFile> =
        requested_files.iter().map(|file| (file.id, file)).collect();

    let mut ctx = TypeMappingContext::new(nodes, file_id_to_file);
    let mut context = ElmContext::new();

    // 处理每个节点
    for node in nodes {
        convert_node_to_elm(node, &mut context, &mut ctx);
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

    let mut ctx = TypeMappingContext::new(rpc_nodes, file_id_to_file);

    for node in rpc_nodes {
        convert_node_to_elm(node, context, &mut ctx);
    }
}

/// 将 Cap'n Proto 节点转换为 Elm 模块
fn convert_node_to_elm(node: &Node, context: &mut ElmContext, ctx: &mut TypeMappingContext) {
    // 直接获取完整模块路径
    let full_module_name = ctx.get_full_type_name(node.id);

    // 提取纯类型名（最后一部分）
    let type_name = TypeMappingContext::extract_type_name(&node.display_name);

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
            let param_type = ctx.map_type(&method.param_type, node.id);
            let result_type = ctx.map_type(&method.result_type, node.id);

            TypeMappingContext::collect_imports_from_type(&param_type, &mut imports);
            TypeMappingContext::collect_imports_from_type(&result_type, &mut imports);
            let param_has_caps = param_type.contains_interface_ref();
            // result_type 是 StructRef 时，contains_interface_ref() 只检查泛型参数
            // 但结果 struct 的字段（如 EchoFactory.create → CreateResults.echo）可能含 interface
            // 需要额外查看对应 struct node 的 fields
            let result_has_caps = result_type.contains_interface_ref()
                || ctx.result_struct_has_interface_fields(&method.result_type);

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
            convert_node_to_elm(nested, context, ctx);
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

            convert_fields(capnp_fields, &mut fields, &mut imports, ctx, node.id);

            if let Some(union_fields) = union_fields {
                let mut branches = Vec::new();
                for field in union_fields {
                    let branch = ElmUnionBranch {
                        name: field.name.clone(),
                        discriminant: field.discriminant.unwrap(),
                        elm_type: ctx.map_type(&field.typ, node.id),
                        offset: field.offset,
                        is_pointer: TypeMappingContext::is_pointer_type(&field.typ),
                        default_value: TypeMappingContext::map_default_value(&field.default_value),
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

                TypeMappingContext::collect_imports_from_type(
                    &unnamed_union.elm_type,
                    &mut imports,
                );

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
                                elm_type: ctx.map_type(&field.typ, node.id),
                                offset: field.offset,
                                is_pointer: TypeMappingContext::is_pointer_type(&field.typ),
                                default_value: TypeMappingContext::map_default_value(
                                    &field.default_value,
                                ),
                            };
                            branches.push(branch);
                        }

                        let named_union = ElmField {
                            name: TypeMappingContext::extract_type_name(&nested.display_name)
                                .to_lower_camel_case(),
                            discriminant: None,
                            elm_type: ElmType::UnionInline(branches, generic_params.clone()),
                            offset: *offset,
                            is_union_container: true,
                            default_value: None,
                        };

                        TypeMappingContext::collect_imports_from_type(
                            &named_union.elm_type,
                            &mut imports,
                        );
                        eprintln!(
                            "DEBUG named_union for {}: branches={:?}",
                            TypeMappingContext::extract_type_name(&nested.display_name),
                            named_union.elm_type
                        );

                        fields.push(named_union);
                    }
                }
            }

            ElmTypeDef::Struct
        }
        NodeKind::Enum(variants) => {
            convert_enum_to_fields(node.id, variants, &mut fields, ctx);
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
    TypeMappingContext::collect_imports_from_defaults(&fields, &mut imports);

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
        convert_node_to_elm(nested, context, ctx);
    }
}

fn convert_fields(
    capnp_fields: &[Field],
    fields: &mut Vec<ElmField>,
    imports: &mut Vec<String>,
    ctx: &mut TypeMappingContext,
    current_node_id: u64,
) {
    for field in capnp_fields {
        if let Type::StructRef(id, _) = field.typ {
            if ctx.is_group_struct(id) {
                continue; // 跳过 group 字段
            }
        }

        let elm_type = ctx.map_type(&field.typ, current_node_id);
        TypeMappingContext::collect_imports_from_type(&elm_type, imports);
        let is_union_container = matches!(elm_type, ElmType::UnionInline(..));

        fields.push(ElmField {
            name: field.name.clone(),
            discriminant: field.discriminant,
            elm_type,
            offset: field.offset,
            is_union_container,
            default_value: TypeMappingContext::map_default_value(&field.default_value),
        });
    }
}

/// 转换枚举为字段
fn convert_enum_to_fields(
    node_id: u64,
    variants: &[crate::capnproto::Enumerator],
    fields: &mut Vec<ElmField>,
    ctx: &mut TypeMappingContext,
) {
    // 枚举作为一个整体字段，包含所有变体信息
    let mut enum_branches = Vec::new();

    for variant in variants {
        enum_branches.push(crate::elm::ElmEnumVariant {
            name: variant.name.clone(),
            ordinal: variant.ordinal,
        });
    }

    let full_name = ctx.get_full_type_name(node_id);

    fields.push(ElmField {
        name: "ignored".to_string(),
        discriminant: None,
        elm_type: ElmType::EnumRef(full_name, "Entity", enum_branches, vec![]),
        offset: 0,
        is_union_container: false,
        default_value: None,
    });
}
