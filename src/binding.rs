use crate::capnproto::{Field, Node, NodeKind, RequestedFile, Type};
use crate::elm::{ElmContext, ElmField, ElmMethod, ElmModule, ElmType, ElmTypeDef, ElmUnionBranch};
use crate::type_mapping::{SynthJob, TypeMappingContext};
use heck::ToLowerCamelCase;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

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

    // 单态化：合成实例化模块（合成过程中可能再遇到新实例化，循环至枯竭）
    while let Some(job) = ctx.pop_pending_synth() {
        build_generic_instantiation_module(&job, &mut context, &mut ctx);
    }

    merge_scc_modules(&mut context);

    propagate_needs_cap_table(&mut context);

    context
}

/// 递归收集类型树中指向其它模块的 struct 引用边。
fn collect_struct_type_edges(
    t: &ElmType,
    current: &str,
    edges: &mut std::collections::HashSet<(String, String)>,
) {
    match t {
        ElmType::StructRef(m, _, args) => {
            if !m.is_empty() && m != current {
                edges.insert((current.to_owned(), m.clone()));
            }
            for a in args {
                collect_struct_type_edges(a, current, edges);
            }
        }
        ElmType::List(inner) => collect_struct_type_edges(inner, current, edges),
        ElmType::UnionInline(branches, _) => {
            for b in branches {
                collect_struct_type_edges(&b.elm_type, current, edges);
            }
        }
        ElmType::EnumRef(_, _, _, args) | ElmType::InterfaceRef(_, _, args) => {
            for a in args {
                collect_struct_type_edges(a, current, edges);
            }
        }
        _ => {}
    }
}

/// 按成员映射改写 StructRef：指向次成员的引用改指规范模块、实体名加前缀。
fn rewrite_scc_refs(t: &mut ElmType, map: &HashMap<String, (String, String)>) {
    match t {
        ElmType::StructRef(m, e, args) => {
            if let Some((canon, sec_name)) = map.get(m.as_str()) {
                // EntityName 是 &'static str：生成器进程短命，leak 可接受
                *e = Box::leak(format!("{sec_name}Entity").into_boxed_str());
                *m = canon.clone();
            }
            for a in args {
                rewrite_scc_refs(a, map);
            }
        }
        ElmType::List(inner) => rewrite_scc_refs(inner, map),
        ElmType::UnionInline(branches, _) => {
            for b in branches {
                rewrite_scc_refs(&mut b.elm_type, map);
            }
        }
        ElmType::EnumRef(_, _, _, args) | ElmType::InterfaceRef(_, _, args) => {
            for a in args {
                rewrite_scc_refs(a, map);
            }
        }
        _ => {}
    }
}

/// SCC 合并：互递归的 struct 模块合并为单个 Elm 模块。
/// Elm 禁止 import 环，而 Cap'n Proto 允许同文件类型互递归
/// （如 Prop.value : JsonValue 与 JsonValue.obj : List(Prop)）。
/// 规范成员取全名字典序最小者，保留原名与全部符号；
/// 次成员渲染期加自身类型名前缀（PropEntity / propDecode / …），
/// 下游 StructRef 重写为（规范模块全名, "次成员名Entity"），imports 同步改写。
pub fn merge_scc_modules(context: &mut ElmContext) {
    // 1) 类型引用边
    let mut edges: std::collections::HashSet<(String, String)> = Default::default();
    for module in &context.modules {
        let current = crate::elm::module_full_name(module);
        for field in &module.fields {
            collect_struct_type_edges(&field.elm_type, &current, &mut edges);
        }
        for method in &module.methods {
            collect_struct_type_edges(&method.param_type, &current, &mut edges);
            collect_struct_type_edges(&method.result_type, &current, &mut edges);
        }
    }
    if edges.is_empty() {
        return;
    }

    // 2) Tarjan SCC（模块规模小，递归足够）
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    for (a, b) in &edges {
        adj.entry(a.clone()).or_default().push(b.clone());
    }
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut low: HashMap<String, usize> = HashMap::new();
    let mut on_stack: std::collections::HashSet<String> = Default::default();
    let mut stack: Vec<String> = vec![];
    let mut counter = 0usize;
    let mut sccs: Vec<Vec<String>> = vec![];

    fn dfs(
        v: &str,
        adj: &HashMap<String, Vec<String>>,
        index: &mut HashMap<String, usize>,
        low: &mut HashMap<String, usize>,
        on_stack: &mut std::collections::HashSet<String>,
        stack: &mut Vec<String>,
        counter: &mut usize,
        sccs: &mut Vec<Vec<String>>,
    ) {
        index.insert(v.to_owned(), *counter);
        low.insert(v.to_owned(), *counter);
        *counter += 1;
        stack.push(v.to_owned());
        on_stack.insert(v.to_owned());
        if let Some(nexts) = adj.get(v) {
            for w in nexts.clone() {
                if !index.contains_key(&w) {
                    dfs(&w, adj, index, low, on_stack, stack, counter, sccs);
                    let wl = low[&w];
                    low.insert(v.to_owned(), low[v].min(wl));
                } else if on_stack.contains(&w) {
                    let wi = index[&w];
                    low.insert(v.to_owned(), low[v].min(wi));
                }
            }
        }
        if low[v] == index[v] {
            let mut comp = vec![];
            while let Some(w) = stack.pop() {
                on_stack.remove(&w);
                let is_root = w == *v;
                comp.push(w);
                if is_root {
                    break;
                }
            }
            if comp.len() > 1 {
                sccs.push(comp);
            }
        }
    }
    let roots: Vec<String> = context
        .modules
        .iter()
        .map(crate::elm::module_full_name)
        .collect();
    for root in &roots {
        if !index.contains_key(root) {
            dfs(
                root,
                &adj,
                &mut index,
                &mut low,
                &mut on_stack,
                &mut stack,
                &mut counter,
                &mut sccs,
            );
        }
    }

    // 3) 逐 SCC 合并
    for comp in sccs {
        let mut members = comp;
        members.sort();
        let canonical = members[0].clone();
        let secondaries: Vec<String> = members[1..].to_vec();
        let member_fulls: std::collections::HashSet<String> = members.iter().cloned().collect();

        // 重写映射：次成员全名 → (规范全名, 次成员类型名)
        let mut map: HashMap<String, (String, String)> = HashMap::new();
        for sec in &secondaries {
            let name = sec.rsplit('.').next().unwrap_or(sec).to_string();
            map.insert(sec.clone(), (canonical.clone(), name));
        }

        // 分区：canonical / 次成员 / 其它
        let mut canon_mod = None;
        let mut sec_mods: Vec<ElmModule> = vec![];
        let mut others: Vec<ElmModule> = vec![];
        for m in std::mem::take(&mut context.modules) {
            let full = crate::elm::module_full_name(&m);
            if full == canonical {
                canon_mod = Some(m);
            } else if member_fulls.contains(&full) {
                sec_mods.push(m);
            } else {
                others.push(m);
            }
        }
        let mut canon_mod = canon_mod.expect("canonical module missing");

        // 守卫：只能合并非泛型 struct 模块
        for m in &sec_mods {
            assert!(
                matches!(m.type_def, ElmTypeDef::Struct) && m.generic_params.is_empty(),
                "SCC 合并遇到非 struct/泛型模块: {}",
                crate::elm::module_full_name(m)
            );
        }

        // 全仓库改写（其它模块 + 规范 + 次成员自身）
        for m in others
            .iter_mut()
            .chain(std::iter::once(&mut canon_mod))
            .chain(sec_mods.iter_mut())
        {
            for field in &mut m.fields {
                rewrite_scc_refs(&mut field.elm_type, &map);
            }
            for method in &mut m.methods {
                rewrite_scc_refs(&mut method.param_type, &map);
                rewrite_scc_refs(&mut method.result_type, &map);
            }
            if crate::elm::module_full_name(m) != canonical {
                for imp in &mut m.imports {
                    if member_fulls.contains(imp) {
                        *imp = canonical.clone();
                    }
                }
                m.imports.sort();
                m.imports.dedup();
            }
        }

        // 规范模块：去掉对次成员的 import（同文件了），挂接次成员
        canon_mod.imports.retain(|i| !member_fulls.contains(i));
        sec_mods.sort_by_key(crate::elm::module_full_name);
        canon_mod.merged_members = sec_mods;

        context.modules = others;
        context.modules.push(canon_mod);
    }
}

/// 该类型作为字段/分支时，getter/decode 是否需要 capTable：
/// interface 直接需要；struct 引用看目标模块（同模块引用回退到当前模块）；
/// 列表看元素；泛型实参递归。
fn type_targets_needs_cap_table(t: &ElmType, needs: &HashSet<String>, current: &str) -> bool {
    match t {
        ElmType::InterfaceRef(..) => true,
        ElmType::StructRef(m, _, args) => {
            let target = if m.is_empty() { current } else { m.as_str() };
            needs.contains(target)
                || args
                    .iter()
                    .any(|a| type_targets_needs_cap_table(a, needs, current))
        }
        ElmType::List(inner) => type_targets_needs_cap_table(inner, needs, current),
        ElmType::UnionInline(branches, _) => branches
            .iter()
            .any(|b| type_targets_needs_cap_table(&b.elm_type, needs, current)),
        _ => false,
    }
}

/// 该类型自身是否携带 capability（含嵌套泛型实参），或指向需要 capTable 的
/// struct 模块（decode 侧的传递闭包）。
fn type_carries_cap_table(t: &ElmType, needs: &HashSet<String>, current: &str) -> bool {
    t.contains_interface_ref() || type_targets_needs_cap_table(t, needs, current)
}

/// 遍历模块及其合并成员的全部字段（合并模块的 capTable 闭包须覆盖成员字段）。
fn fields_including_merged(
    module: &mut ElmModule,
) -> impl Iterator<Item = &mut ElmField> {
    module.fields.iter_mut().chain(
        module
            .merged_members
            .iter_mut()
            .flat_map(|m| m.fields.iter_mut()),
    )
}

/// 计算 decode 侧的 capTable 需求闭包：
/// 1. 直接含 interface 字段的模块需要（contains_interface_ref 递归含 union 分支）；
/// 2. 引用（字段/union 分支/列表元素）了需要 capTable 的 struct 模块的模块
///    同样需要 —— decode 必须向下传 capTable；
/// 3. 在每个 union 分支与非 union 字段上盖 needs_cap_table 章，供模板决定
///    getter 签名与 decode 调用是否携带 capTable；
/// 4. 接口方法 result_has_caps 按同一闭包重算（decode 结果的调用点在 rpc 模板）。
/// param_has_caps 不在此列：它对齐的是 encodeWithCaps 的存在性（直接字段判定）。
pub fn propagate_needs_cap_table(context: &mut ElmContext) {
    // 1) 直接判定（合并成员的字段一并计入规范模块）
    for module in &mut context.modules {
        let needs = fields_including_merged(module)
            .any(|f| f.elm_type.contains_interface_ref());
        module.needs_cap_table = needs;
    }

    // 2) 沿 struct 引用传递至不动点（每轮用需求集快照，避免借用冲突）
    loop {
        let needs: HashSet<String> = context
            .modules
            .iter()
            .filter(|m| m.needs_cap_table)
            .map(crate::elm::module_full_name)
            .collect();
        let mut changed = false;
        for module in &mut context.modules {
            if module.needs_cap_table {
                continue;
            }
            let current = crate::elm::module_full_name(module);
            if fields_including_merged(module).any(|f| {
                type_targets_needs_cap_table(&f.elm_type, &needs, &current)
            }) {
                module.needs_cap_table = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // 3) 逐分支/逐字段盖章
    let needs: HashSet<String> = context
        .modules
        .iter()
        .filter(|m| m.needs_cap_table)
        .map(crate::elm::module_full_name)
        .collect();
    for module in &mut context.modules {
        // 传递性模块（自身无 interface 字段但分支/字段需要 capTable）也要 import Rpc
        if module.needs_cap_table && !module.imports.iter().any(|i| i == "Rpc.Client as Rpc") {
            module.imports.push("Rpc.Client as Rpc".to_owned());
        }
        let current = crate::elm::module_full_name(module);
        for field in fields_including_merged(module) {
            if field.is_union_container {
                if let ElmType::UnionInline(branches, _) = &mut field.elm_type {
                    for branch in branches {
                        branch.needs_cap_table =
                            type_targets_needs_cap_table(&branch.elm_type, &needs, &current);
                    }
                }
            } else {
                field.needs_cap_table =
                    type_targets_needs_cap_table(&field.elm_type, &needs, &current);
            }
        }
    }

    // 4) 接口方法 result 侧按同一闭包重算
    for module in &mut context.modules {
        let current = crate::elm::module_full_name(module);
        for method in &mut module.methods {
            method.result_has_caps = type_carries_cap_table(&method.result_type, &needs, &current);
        }
    }
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

/// struct 节点共用的内容构建：常规字段 + 匿名/内联 union。
/// convert_node_to_elm 与单态化合成共用此入口。
struct BuiltStruct {
    type_def: ElmTypeDef,
    fields: Vec<ElmField>,
    imports: Vec<String>,
    data_words: u32,
    pointer_words: u32,
    discriminant_offset: u32,
}

fn build_struct_contents(node: &Node, ctx: &mut TypeMappingContext) -> BuiltStruct {
    let mut imports = vec![];
    let mut fields = Vec::new();
    let generic_params: Vec<String> = node
        .generic_params
        .iter()
        .map(|x| x.to_lowercase())
        .collect();

    let NodeKind::Struct {
        is_group: _,
        fields: capnp_fields,
        data_word_count,
        pointer_word_count,
        union_fields,
        discriminant_offset: offset,
    } = &node.kind
    else {
        unreachable!("build_struct_contents 只接受 struct 节点")
    };

    // 判别式偏移：顶层匿名 union 用外层节点值；仅含命名 union 的 struct
    // （如 rpc.capnp Call/Disembargo）必须在下方从嵌套 group 节点覆盖，
    // 否则 getWhich/writeUInt16 的 tag 偏移错成 0，破坏线格式。
    let mut discriminant_offset = *offset;

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
                needs_cap_table: false,
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
            cap_slot: None,
            needs_cap_table: false,
        };

        TypeMappingContext::collect_imports_from_type(&unnamed_union.elm_type, &mut imports);

        fields.push(unnamed_union);
    } else {
        // 处理内联的联合体节点（有命名的内嵌union）
        for nested in &node.nested_nodes {
            if let NodeKind::Struct {
                is_group: true,
                union_fields: Some(capnp_fields),
                discriminant_offset: offset,
                ..
            } = &nested.kind
            {
                let mut branches = Vec::new();
                for field in capnp_fields {
                    let branch = ElmUnionBranch {
                        name: field.name.clone(),
                        discriminant: field.discriminant.unwrap_or(0),
                        elm_type: ctx.map_type(&field.typ, node.id),
                        offset: field.offset,
                        is_pointer: TypeMappingContext::is_pointer_type(&field.typ),
                        default_value: TypeMappingContext::map_default_value(&field.default_value),
                        needs_cap_table: false,
                    };
                    branches.push(branch);
                }

                // 命名 union 的 tag 偏移属于嵌套节点，覆盖顶层值
                discriminant_offset = *offset;

                let named_union = ElmField {
                    name: TypeMappingContext::extract_type_name(&nested.display_name)
                        .to_lower_camel_case(),
                    discriminant: None,
                    elm_type: ElmType::UnionInline(branches, generic_params.clone()),
                    offset: discriminant_offset,
                    is_union_container: true,
                    default_value: None,
                    cap_slot: None,
                    needs_cap_table: false,
                };

                TypeMappingContext::collect_imports_from_type(&named_union.elm_type, &mut imports);
                fields.push(named_union);
            }
        }
    }

    BuiltStruct {
        type_def: ElmTypeDef::Struct,
        fields,
        imports,
        data_words: *data_word_count as u32,
        pointer_words: *pointer_word_count as u32,
        discriminant_offset,
    }
}

/// 单态化：为泛型实例化合成具体 struct 模块（如 Docs….Lookup.OfJsonValue）。
/// 合成期间 type_param_env 生效，GenericParam 直接替换为已映射的具体实参。
fn build_generic_instantiation_module(
    job: &SynthJob,
    context: &mut ElmContext,
    ctx: &mut TypeMappingContext,
) {
    if !matches!(job.node.kind, NodeKind::Struct { .. }) {
        return; // v1 只单态化泛型 struct
    }

    ctx.type_param_env = Some(job.env.clone());
    let built = build_struct_contents(&job.node, ctx);
    ctx.type_param_env = None;

    let mut imports = built.imports;
    let full_name = format!("{}.{}.{}", job.parent_path, job.generic_last, job.mangled);
    imports.retain(|import| import != &full_name);
    // 合成模块同样生成 encode/decode,恒定依赖 Capnproto 运行时
    imports.push("Capnproto".to_owned());
    imports.sort();
    imports.dedup();

    // 合成模块 id：dedup_key 哈希，避免与原节点/其他实例化冲突
    let mut hasher = DefaultHasher::new();
    job.dedup_key.hash(&mut hasher);

    let path = if job.parent_path.is_empty() {
        job.generic_last.clone()
    } else {
        format!("{}.{}", job.parent_path, job.generic_last)
    };

    context.modules.push(ElmModule {
        id: hasher.finish(),
        name: job.mangled.clone(),
        path,
        imports,
        type_def: built.type_def,
        data_words: built.data_words,
        pointer_words: built.pointer_words,
        fields: built.fields,
        discriminant_offset: built.discriminant_offset,
        methods: vec![],
        generic_params: vec![],
        needs_cap_table: false,
        merged_members: vec![],
    });
}

/// 将 Cap'n Proto 节点转换为 Elm 模块
fn convert_node_to_elm(node: &Node, context: &mut ElmContext, ctx: &mut TypeMappingContext) {
    // 单态化：泛型定义本身不再产出参数化模块（实例化在 drain 循环中合成为具体模块）
    if !node.generic_params.is_empty() {
        return;
    }

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
            // param_has_caps 必须与 struct 模块会生成 encodeWithCaps 的判定一致
            // （即参数 struct 的字段含 interface），否则生成的调用会指向不存在的函数。
            // 注意不要用 param_type.contains_interface_ref()：泛型参数含 interface 时
            // 参数 struct 模块本身并不生成 encodeWithCaps。
            let param_has_caps = ctx.struct_type_has_interface_fields(&method.param_type);
            // param/result_type 是 StructRef 时，contains_interface_ref() 只检查泛型参数
            // 但 struct 的字段（如 EchoFactory.create → CreateResults.echo）可能含 interface
            // 需要额外查看对应 struct node 的 fields
            let result_has_caps = result_type.contains_interface_ref()
                || ctx.struct_type_has_interface_fields(&method.result_type);

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
            needs_cap_table: false,
            merged_members: vec![],
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
        NodeKind::Struct { .. } => {
            let built = build_struct_contents(node, ctx);
            fields = built.fields;
            imports = built.imports;
            data_words = built.data_words;
            pointer_words = built.pointer_words;
            discriminant_offset = built.discriminant_offset;
            built.type_def
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
        needs_cap_table: false,
        merged_members: vec![],
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
    // capability 字段按声明顺序分配 payload capTable 槽位
    let mut next_cap_slot = 0;
    for field in capnp_fields {
        if let Type::StructRef(id, _) = field.typ {
            if ctx.is_group_struct(id) {
                continue; // 跳过 group 字段
            }
        }

        let elm_type = ctx.map_type(&field.typ, current_node_id);
        TypeMappingContext::collect_imports_from_type(&elm_type, imports);
        let is_union_container = matches!(elm_type, ElmType::UnionInline(..));
        let cap_slot = if elm_type.is_interface_ref() {
            let slot = next_cap_slot;
            next_cap_slot += 1;
            Some(slot)
        } else {
            None
        };

        fields.push(ElmField {
            name: field.name.clone(),
            discriminant: field.discriminant,
            elm_type,
            offset: field.offset,
            is_union_container,
            default_value: TypeMappingContext::map_default_value(&field.default_value),
            cap_slot,
            needs_cap_table: false,
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
        cap_slot: None,
        needs_cap_table: false,
    });
}
