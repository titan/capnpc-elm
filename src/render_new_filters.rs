// These are the new filter functions to append to the filters mod block

    /// Returns true if the list element type is Int
    pub fn is_int_list(elm_type: &ElmType) -> askama::Result<bool> {
        match elm_type {
            ElmType::List(inner) => {
                Ok(matches!(inner.as_ref(), ElmType::Primitive(ElmPrimitiveType::Int(_))))
            }
            _ => Ok(false),
        }
    }

    /// Returns true if the list element type is Bool
    pub fn is_bool_list(elm_type: &ElmType) -> askama::Result<bool> {
        match elm_type {
            ElmType::List(inner) => {
                Ok(matches!(inner.as_ref(), ElmType::Primitive(ElmPrimitiveType::Bool)))
            }
            _ => Ok(false),
        }
    }

    /// Returns true if the list element type is Float
    pub fn is_float_list(elm_type: &ElmType) -> askama::Result<bool> {
        match elm_type {
            ElmType::List(inner) => {
                Ok(matches!(inner.as_ref(), ElmType::Primitive(ElmPrimitiveType::Float(_))))
            }
            _ => Ok(false),
        }
    }

    /// Returns true if the list element type is Text (String)
    pub fn is_text_list(elm_type: &ElmType) -> askama::Result<bool> {
        match elm_type {
            ElmType::List(inner) => {
                Ok(matches!(inner.as_ref(), ElmType::Primitive(ElmPrimitiveType::String)))
            }
            _ => Ok(false),
        }
    }

    /// Returns the full list encoder expression for use in templates
    pub fn list_encoder_expr(elm_type: &ElmType) -> askama::Result<String> {
        match elm_type {
            ElmType::List(inner) => {
                match inner.as_ref() {
                    ElmType::Primitive(ElmPrimitiveType::Int(8)) => Ok("Capnproto.encodePrimitiveIntList 1 1".to_string()),
                    ElmType::Primitive(ElmPrimitiveType::Int(16)) => Ok("Capnproto.encodePrimitiveIntList 3 2".to_string()),
                    ElmType::Primitive(ElmPrimitiveType::Int(32)) => Ok("Capnproto.encodePrimitiveIntList 4 4".to_string()),
                    ElmType::Primitive(ElmPrimitiveType::Int(64)) => Ok("Capnproto.encodePrimitiveIntList 5 8".to_string()),
                    ElmType::Primitive(ElmPrimitiveType::Bool) => Ok("Capnproto.encodeBoolList".to_string()),
                    ElmType::Primitive(ElmPrimitiveType::Float(32)) => Ok("Capnproto.encodeFloat32List".to_string()),
                    ElmType::Primitive(ElmPrimitiveType::Float(64)) => Ok("Capnproto.encodeFloat64List".to_string()),
                    ElmType::Primitive(ElmPrimitiveType::String) => Ok("Capnproto.encodeTextList".to_string()),
                    ElmType::StructRef(module_name, _, _) => {
                        if module_name.is_empty() {
                            Ok("Capnproto.encodeStructList encode dataWords pointerWords".to_string())
                        } else {
                            Ok(format!("Capnproto.encodeStructList {}.encode {}.dataWords {}.pointerWords", module_name, module_name, module_name))
                        }
                    }
                    _ => Ok("Capnproto.encodeStructList encode dataWords pointerWords".to_string()),
                }
            }
            _ => Ok("Capnproto.encodeStructList encode dataWords pointerWords".to_string()),
        }
    }

    /// Returns the list element decoder expression for getXAt functions
    pub fn list_element_reader_expr(elm_type: &ElmType) -> askama::Result<String> {
        match elm_type {
            ElmType::List(inner) => {
                match inner.as_ref() {
                    ElmType::Primitive(ElmPrimitiveType::Int(8)) => Ok("\\r -> Capnproto.readUInt8 r 0".to_string()),
                    ElmType::Primitive(ElmPrimitiveType::Int(16)) => Ok("\\r -> Capnproto.readUInt16 r 0".to_string()),
                    ElmType::Primitive(ElmPrimitiveType::Int(32)) => Ok("\\r -> Capnproto.readUInt32 r 0".to_string()),
                    ElmType::Primitive(ElmPrimitiveType::Int(64)) => Ok("\\r -> Capnproto.readUInt64 r 0".to_string()),
                    ElmType::Primitive(ElmPrimitiveType::Bool) => Ok("\\r -> Capnproto.readBool r 0 0".to_string()),
                    ElmType::Primitive(ElmPrimitiveType::Float(32)) => Ok("\\r -> Capnproto.readFloat32 r 0".to_string()),
                    ElmType::Primitive(ElmPrimitiveType::Float(64)) => Ok("\\r -> Capnproto.readFloat64 r 0".to_string()),
                    ElmType::Primitive(ElmPrimitiveType::String) => Ok("\\r -> Capnproto.readText r 0".to_string()),
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
                            Ok(format!("\\r -> Capnproto.readUInt16 r 0 |> Maybe.andThen {}.fromCode", module_name))
                        }
                    }
                    _ => Ok("\\r -> Nothing".to_string()),
                }
            }
            _ => Ok("\\r -> Nothing".to_string()),
        }
    }
