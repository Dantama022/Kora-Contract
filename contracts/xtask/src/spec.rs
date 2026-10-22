//! Real Soroban WASM/XDR Contract Specification Extraction (#937).
//!
//! Extracts contract specifications from compiled WASM custom sections (`contractspecv0`)
//! using `soroban-spec` and `stellar-xdr`. Generates deterministic JSON and formatted
//! signatures for interface-compatibility checks and SDK type generation.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::Path;
use stellar_xdr::curr::{ScSpecEntry, ScSpecTypeDef, ScSpecUdtUnionCaseV0};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractSpec {
    pub name: String,
    pub version: String,
    pub functions: Vec<FunctionSpec>,
    pub types: Vec<TypeSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub structs: Vec<StructSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unions: Vec<UnionSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enums: Vec<EnumSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ErrorSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionSpec {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    pub is_readonly: bool,
    pub parameters: Vec<ParameterSpec>,
    pub return_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParameterSpec {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    pub param_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TypeSpec {
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    pub fields: Vec<TypeField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TypeField {
    pub name: String,
    pub field_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructSpec {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    pub fields: Vec<TypeField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnionCaseSpec {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    pub tuple_types: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnionSpec {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    pub cases: Vec<UnionCaseSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnumCaseSpec {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    pub value: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnumSpec {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    pub cases: Vec<EnumCaseSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorSpec {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    pub cases: Vec<EnumCaseSpec>,
}

#[derive(Debug)]
pub enum SpecError {
    Io(std::io::Error),
    WasmRead(String),
    MissingSpecSection,
    MalformedWasm(String),
    Json(serde_json::Error),
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::WasmRead(e) => write!(f, "WASM spec read error: {e}"),
            Self::MissingSpecSection => write!(f, "WASM does not contain a contractspecv0 section"),
            Self::MalformedWasm(e) => write!(f, "Malformed WASM file: {e}"),
            Self::Json(e) => write!(f, "JSON serialization error: {e}"),
        }
    }
}

impl std::error::Error for SpecError {}

impl From<std::io::Error> for SpecError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for SpecError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

/// Helper to convert ScSpecTypeDef to a clean, canonical string.
/// Never panics on unrecognized or future variants.
pub fn type_def_to_string(type_def: &ScSpecTypeDef) -> String {
    match type_def {
        ScSpecTypeDef::Val => "Val".to_string(),
        ScSpecTypeDef::U64 => "u64".to_string(),
        ScSpecTypeDef::I64 => "i64".to_string(),
        ScSpecTypeDef::U128 => "u128".to_string(),
        ScSpecTypeDef::I128 => "i128".to_string(),
        ScSpecTypeDef::U32 => "u32".to_string(),
        ScSpecTypeDef::I32 => "i32".to_string(),
        ScSpecTypeDef::U256 => "u256".to_string(),
        ScSpecTypeDef::I256 => "i256".to_string(),
        ScSpecTypeDef::Bool => "bool".to_string(),
        ScSpecTypeDef::Void => "void".to_string(),
        ScSpecTypeDef::Error => "Error".to_string(),
        ScSpecTypeDef::String => "String".to_string(),
        ScSpecTypeDef::Symbol => "Symbol".to_string(),
        ScSpecTypeDef::Bytes => "Bytes".to_string(),
        ScSpecTypeDef::Address => "Address".to_string(),
        ScSpecTypeDef::Timepoint => "Timepoint".to_string(),
        ScSpecTypeDef::Duration => "Duration".to_string(),
        ScSpecTypeDef::Option(opt) => format!("Option<{}>", type_def_to_string(&opt.value_type)),
        ScSpecTypeDef::Result(res) => format!(
            "Result<{}, {}>",
            type_def_to_string(&res.ok_type),
            type_def_to_string(&res.error_type)
        ),
        ScSpecTypeDef::Vec(v) => format!("Vec<{}>", type_def_to_string(&v.element_type)),
        ScSpecTypeDef::Map(m) => format!(
            "Map<{}, {}>",
            type_def_to_string(&m.key_type),
            type_def_to_string(&m.value_type)
        ),
        ScSpecTypeDef::Tuple(t) => {
            let inner = t
                .value_types
                .iter()
                .map(type_def_to_string)
                .collect::<Vec<_>>()
                .join(", ");
            format!("({inner})")
        }
        ScSpecTypeDef::BytesN(b) => format!("BytesN<{}>", b.n),
        ScSpecTypeDef::Udt(u) => u
            .name
            .to_utf8_string()
            .unwrap_or_else(|_| u.name.to_utf8_string_lossy()),
    }
}

/// Heuristic for read-only functions in Soroban contracts
pub fn is_readonly_function_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with("get_")
        || lower.starts_with("is_")
        || lower.starts_with("has_")
        || lower.starts_with("check_")
        || lower.starts_with("query_")
        || lower.starts_with("read_")
        || lower.starts_with("balance_")
        || lower.starts_with("total_")
        || lower == "balance"
        || lower == "decimals"
        || lower == "name"
        || lower == "symbol"
        || lower == "owner"
        || lower == "admin"
        || lower == "version"
}

fn clean_doc_string(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Extract contract spec from compiled WASM bytes.
pub fn extract_spec_from_wasm(wasm_bytes: &[u8], name_override: Option<&str>) -> Result<ContractSpec, SpecError> {
    if wasm_bytes.len() < 8 || &wasm_bytes[0..4] != b"\0asm" {
        return Err(SpecError::MalformedWasm("Invalid WASM magic header".to_string()));
    }

    let entries = soroban_spec::read::from_wasm(wasm_bytes).map_err(|e| {
        let msg = format!("{e}");
        if msg.contains("custom section") || msg.contains("not found") {
            SpecError::MissingSpecSection
        } else {
            SpecError::WasmRead(msg)
        }
    })?;

    if entries.is_empty() {
        return Err(SpecError::MissingSpecSection);
    }

    let mut functions = Vec::new();
    let mut structs = Vec::new();
    let mut unions = Vec::new();
    let mut enums = Vec::new();
    let mut errors = Vec::new();
    let mut types = Vec::new();

    for entry in entries {
        match entry {
            ScSpecEntry::FunctionV0(f) => {
                let name = f.name.to_utf8_string().unwrap_or_else(|_| f.name.to_utf8_string_lossy());
                let is_readonly = is_readonly_function_name(&name);
                let parameters: Vec<ParameterSpec> = f
                    .inputs
                    .iter()
                    .map(|input| ParameterSpec {
                        name: input
                            .name
                            .to_utf8_string()
                            .unwrap_or_else(|_| input.name.to_utf8_string_lossy()),
                        doc: input
                            .doc
                            .to_utf8_string()
                            .ok()
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty()),
                        param_type: type_def_to_string(&input.type_),
                    })
                    .collect();

                let return_type = if f.outputs.is_empty() {
                    None
                } else if f.outputs.len() == 1 {
                    let t = type_def_to_string(&f.outputs[0]);
                    if t == "void" {
                        None
                    } else {
                        Some(t)
                    }
                } else {
                    let tuple_inner = f
                        .outputs
                        .iter()
                        .map(type_def_to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    Some(format!("({tuple_inner})"))
                };

                functions.push(FunctionSpec {
                    name,
                    doc: f.doc.to_utf8_string().ok().and_then(|s| clean_doc_string(&s)),
                    is_readonly,
                    parameters,
                    return_type,
                });
            }
            ScSpecEntry::UdtStructV0(s) => {
                let name = s.name.to_utf8_string().unwrap_or_else(|_| s.name.to_utf8_string_lossy());
                let fields: Vec<TypeField> = s
                    .fields
                    .iter()
                    .map(|field| TypeField {
                        name: field
                            .name
                            .to_utf8_string()
                            .unwrap_or_else(|_| field.name.to_utf8_string_lossy()),
                        field_type: type_def_to_string(&field.type_),
                        doc: field
                            .doc
                            .to_utf8_string()
                            .ok()
                            .map(|st| st.trim().to_string())
                            .filter(|st| !st.is_empty()),
                    })
                    .collect();

                let doc = s.doc.to_utf8_string().ok().and_then(|st| clean_doc_string(&st));
                types.push(TypeSpec {
                    name: name.clone(),
                    kind: "struct".to_string(),
                    doc: doc.clone(),
                    fields: fields.clone(),
                });

                structs.push(StructSpec {
                    name,
                    doc,
                    fields,
                });
            }
            ScSpecEntry::UdtUnionV0(u) => {
                let name = u.name.to_utf8_string().unwrap_or_else(|_| u.name.to_utf8_string_lossy());
                let mut cases = Vec::new();
                let mut type_fields = Vec::new();

                for c in u.cases.iter() {
                    match c {
                        ScSpecUdtUnionCaseV0::VoidV0(v) => {
                            let case_name = v
                                .name
                                .to_utf8_string()
                                .unwrap_or_else(|_| v.name.to_utf8_string_lossy());
                            type_fields.push(TypeField {
                                name: case_name.clone(),
                                field_type: "void".to_string(),
                                doc: v
                                    .doc
                                    .to_utf8_string()
                                    .ok()
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty()),
                            });
                            cases.push(UnionCaseSpec {
                                name: case_name,
                                doc: v
                                    .doc
                                    .to_utf8_string()
                                    .ok()
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty()),
                                tuple_types: vec![],
                            });
                        }
                        ScSpecUdtUnionCaseV0::TupleV0(t) => {
                            let case_name = t
                                .name
                                .to_utf8_string()
                                .unwrap_or_else(|_| t.name.to_utf8_string_lossy());
                            let tuple_types: Vec<String> =
                                t.type_.iter().map(type_def_to_string).collect();
                            let field_type = if tuple_types.is_empty() {
                                "void".to_string()
                            } else if tuple_types.len() == 1 {
                                tuple_types[0].clone()
                            } else {
                                format!("({})", tuple_types.join(", "))
                            };
                            type_fields.push(TypeField {
                                name: case_name.clone(),
                                field_type,
                                doc: t
                                    .doc
                                    .to_utf8_string()
                                    .ok()
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty()),
                            });
                            cases.push(UnionCaseSpec {
                                name: case_name,
                                doc: t
                                    .doc
                                    .to_utf8_string()
                                    .ok()
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty()),
                                tuple_types,
                            });
                        }
                    }
                }

                let doc = u.doc.to_utf8_string().ok().and_then(|st| clean_doc_string(&st));
                types.push(TypeSpec {
                    name: name.clone(),
                    kind: "union".to_string(),
                    doc: doc.clone(),
                    fields: type_fields,
                });

                unions.push(UnionSpec {
                    name,
                    doc,
                    cases,
                });
            }
            ScSpecEntry::UdtEnumV0(e) => {
                let name = e.name.to_utf8_string().unwrap_or_else(|_| e.name.to_utf8_string_lossy());
                let cases: Vec<EnumCaseSpec> = e
                    .cases
                    .iter()
                    .map(|case| EnumCaseSpec {
                        name: case
                            .name
                            .to_utf8_string()
                            .unwrap_or_else(|_| case.name.to_utf8_string_lossy()),
                        doc: case
                            .doc
                            .to_utf8_string()
                            .ok()
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty()),
                        value: case.value,
                    })
                    .collect();

                let fields = cases
                    .iter()
                    .map(|c| TypeField {
                        name: c.name.clone(),
                        field_type: format!("{}", c.value),
                        doc: c.doc.clone(),
                    })
                    .collect();

                let doc = e.doc.to_utf8_string().ok().and_then(|st| clean_doc_string(&st));
                types.push(TypeSpec {
                    name: name.clone(),
                    kind: "enum".to_string(),
                    doc: doc.clone(),
                    fields,
                });

                enums.push(EnumSpec {
                    name,
                    doc,
                    cases,
                });
            }
            ScSpecEntry::UdtErrorEnumV0(err) => {
                let name = err.name.to_utf8_string().unwrap_or_else(|_| err.name.to_utf8_string_lossy());
                let cases: Vec<EnumCaseSpec> = err
                    .cases
                    .iter()
                    .map(|case| EnumCaseSpec {
                        name: case
                            .name
                            .to_utf8_string()
                            .unwrap_or_else(|_| case.name.to_utf8_string_lossy()),
                        doc: case
                            .doc
                            .to_utf8_string()
                            .ok()
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty()),
                        value: case.value,
                    })
                    .collect();

                let fields = cases
                    .iter()
                    .map(|c| TypeField {
                        name: c.name.clone(),
                        field_type: format!("{}", c.value),
                        doc: c.doc.clone(),
                    })
                    .collect();

                let doc = err.doc.to_utf8_string().ok().and_then(|st| clean_doc_string(&st));
                types.push(TypeSpec {
                    name: name.clone(),
                    kind: "error_enum".to_string(),
                    doc: doc.clone(),
                    fields,
                });

                errors.push(ErrorSpec {
                    name,
                    doc,
                    cases,
                });
            }
        }
    }

    // Sort all arrays deterministically
    functions.sort_by(|a, b| a.name.cmp(&b.name));
    types.sort_by(|a, b| a.name.cmp(&b.name));
    structs.sort_by(|a, b| a.name.cmp(&b.name));
    unions.sort_by(|a, b| a.name.cmp(&b.name));
    enums.sort_by(|a, b| a.name.cmp(&b.name));
    errors.sort_by(|a, b| a.name.cmp(&b.name));

    let name = name_override
        .unwrap_or_else(|| {
            // derive from first type or fallback
            structs
                .first()
                .map(|s| s.name.as_str())
                .unwrap_or("contract")
        })
        .to_string();

    Ok(ContractSpec {
        name,
        version: "1.0.0".to_string(),
        functions,
        types,
        structs,
        unions,
        enums,
        errors,
    })
}

/// Extract contract spec from a file path
pub fn extract_spec_from_file<P: AsRef<Path>>(wasm_path: P) -> Result<ContractSpec, SpecError> {
    let path = wasm_path.as_ref();
    let wasm_bytes = fs::read(path)?;
    let contract_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.trim_start_matches("kora_").trim_start_matches("lib"));
    extract_spec_from_wasm(&wasm_bytes, contract_name)
}

/// Extract multiple contract specs from multiple WASM files
pub fn extract_specs_from_files<P: AsRef<Path>>(wasm_paths: &[P]) -> Result<Vec<ContractSpec>, SpecError> {
    let mut specs = Vec::new();
    for p in wasm_paths {
        specs.push(extract_spec_from_file(p)?);
    }
    specs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(specs)
}

/// Compatibility fallback for existing caller
pub fn extract_contract_spec(path: &Path) -> Result<ContractSpec, Box<dyn std::error::Error>> {
    let bytes = fs::read(path)?;
    if bytes.starts_with(b"\0asm") {
        Ok(extract_spec_from_wasm(&bytes, path.file_stem().and_then(|s| s.to_str()))?)
    } else {
        // Parse as JSON spec if text
        let content = String::from_utf8(bytes)?;
        let spec: ContractSpec = serde_json::from_str(&content)?;
        Ok(spec)
    }
}

/// Validate that extracted spec matches expected structure.
pub fn validate_spec(spec: &ContractSpec) -> Result<(), String> {
    if spec.name.is_empty() {
        return Err("Contract name must not be empty".to_string());
    }
    if spec.version.is_empty() {
        return Err("Contract version must not be empty".to_string());
    }
    Ok(())
}

/// Formats the extracted spec into deterministic JSON.
pub fn spec_to_deterministic_json(spec: &ContractSpec) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(spec)
}

/// Formats contract functions and signatures into clean text representation
/// suitable for diff-based interface compatibility checks.
pub fn format_spec_interface_text(spec: &ContractSpec) -> String {
    let mut out = String::new();
    out.push_str(&format!("contract {}\n", spec.name));
    out.push_str("functions:\n");
    for f in &spec.functions {
        let params: Vec<String> = f
            .parameters
            .iter()
            .map(|p| format!("{}: {}", p.name, p.param_type))
            .collect();
        let ret = f
            .return_type
            .as_deref()
            .map(|r| format!(" -> {r}"))
            .unwrap_or_default();
        out.push_str(&format!("  fn {}({}){}\n", f.name, params.join(", "), ret));
    }

    if !spec.types.is_empty() {
        out.push_str("types:\n");
        for t in &spec.types {
            out.push_str(&format!("  {} {}:\n", t.kind, t.name));
            for f in &t.fields {
                out.push_str(&format!("    {}: {}\n", f.name, f.field_type));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contract_spec_serialization() {
        let spec = ContractSpec {
            name: "marketplace".to_string(),
            version: "1.0.0".to_string(),
            functions: vec![
                FunctionSpec {
                    name: "list_invoice".to_string(),
                    doc: None,
                    is_readonly: true,
                    parameters: vec![ParameterSpec {
                        name: "id".to_string(),
                        doc: None,
                        param_type: "u64".to_string(),
                    }],
                    return_type: Some("Invoice".to_string()),
                },
                FunctionSpec {
                    name: "create_listing".to_string(),
                    doc: None,
                    is_readonly: false,
                    parameters: vec![
                        ParameterSpec {
                            name: "invoice_id".to_string(),
                            doc: None,
                            param_type: "u64".to_string(),
                        },
                        ParameterSpec {
                            name: "price".to_string(),
                            doc: None,
                            param_type: "i128".to_string(),
                        },
                    ],
                    return_type: None,
                },
            ],
            types: vec![TypeSpec {
                name: "Invoice".to_string(),
                kind: "struct".to_string(),
                doc: None,
                fields: vec![
                    TypeField {
                        name: "id".to_string(),
                        field_type: "u64".to_string(),
                        doc: None,
                    },
                    TypeField {
                        name: "amount".to_string(),
                        field_type: "i128".to_string(),
                        doc: None,
                    },
                ],
            }],
            structs: vec![],
            unions: vec![],
            enums: vec![],
            errors: vec![],
        };

        let json = spec_to_deterministic_json(&spec).expect("serialization failed");
        let parsed: ContractSpec = serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(parsed.name, "marketplace");
        assert_eq!(parsed.version, "1.0.0");
        assert_eq!(parsed.functions.len(), 2);
        assert_eq!(parsed.types.len(), 1);
    }

    #[test]
    fn test_validate_spec() {
        let mut spec = ContractSpec {
            name: "".to_string(),
            version: "1.0.0".to_string(),
            functions: vec![],
            types: vec![],
            structs: vec![],
            unions: vec![],
            enums: vec![],
            errors: vec![],
        };
        assert!(validate_spec(&spec).is_err());
        spec.name = "invoice".to_string();
        spec.version = "".to_string();
        assert!(validate_spec(&spec).is_err());
        spec.version = "1.0.0".to_string();
        assert!(validate_spec(&spec).is_ok());
    }

    #[test]
    fn test_malformed_wasm() {
        let bad_bytes = b"not a wasm file";
        let res = extract_spec_from_wasm(bad_bytes, None);
        assert!(matches!(res, Err(SpecError::MalformedWasm(_))));
    }

    #[test]
    fn test_missing_spec_section() {
        // Minimal valid wasm module: \0asm\1\0\0\0 without custom sections
        let minimal_wasm = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        let res = extract_spec_from_wasm(&minimal_wasm, None);
        assert!(matches!(res, Err(SpecError::MissingSpecSection)));
    }

    #[test]
    fn test_type_def_to_string_all_types() {
        assert_eq!(type_def_to_string(&ScSpecTypeDef::U32), "u32");
        assert_eq!(type_def_to_string(&ScSpecTypeDef::I128), "i128");
        assert_eq!(type_def_to_string(&ScSpecTypeDef::Address), "Address");
        assert_eq!(type_def_to_string(&ScSpecTypeDef::Bytes), "Bytes");
        assert_eq!(type_def_to_string(&ScSpecTypeDef::String), "String");
        assert_eq!(type_def_to_string(&ScSpecTypeDef::Bool), "bool");
        assert_eq!(type_def_to_string(&ScSpecTypeDef::Void), "void");
    }

    #[test]
    fn test_extract_real_invoice_nft_wasm() {
        let candidates = [
            Path::new("target/wasm32-unknown-unknown/release/kora_invoice_nft.wasm"),
            Path::new("../../target/wasm32-unknown-unknown/release/kora_invoice_nft.wasm"),
        ];
        let wasm_path = match candidates.iter().find(|p| p.exists()) {
            Some(p) => *p,
            None => {
                eprintln!("Skipping real wasm extraction test: invoice_nft wasm not built");
                return;
            }
        };

        let spec = extract_spec_from_file(wasm_path).expect("Failed to extract spec from invoice_nft wasm");
        assert!(!spec.functions.is_empty(), "invoice_nft should export functions");

        // Verify key exported functions exist
        let fn_names: Vec<&str> = spec.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(fn_names.contains(&"mint_invoice"), "Should export mint_invoice");
        assert!(fn_names.contains(&"get_invoice"), "Should export get_invoice");
        assert!(fn_names.contains(&"set_funded"), "Should export set_funded");
        assert!(fn_names.contains(&"set_repaid"), "Should export set_repaid");

        // Verify readonly detection
        let get_inv = spec.functions.iter().find(|f| f.name == "get_invoice").unwrap();
        assert!(get_inv.is_readonly, "get_invoice should be marked readonly");

        // Format to interface text
        let iface = format_spec_interface_text(&spec);
        assert!(iface.contains("fn mint_invoice("));
        assert!(iface.contains("fn get_invoice("));

        // Golden file comparison
        let fixture_candidates = [
            Path::new("contracts/xtask/tests/fixtures/invoice_nft.spec.json"),
            Path::new("tests/fixtures/invoice_nft.spec.json"),
        ];
        if let Some(fixture_path) = fixture_candidates.iter().find(|p| p.exists()) {
            let fixture_json = fs::read_to_string(fixture_path).expect("Failed to read golden fixture");
            let golden_spec: ContractSpec = serde_json::from_str(&fixture_json).expect("Failed to parse golden fixture");
            assert_eq!(spec.name, golden_spec.name, "Golden test spec name mismatch");
            assert_eq!(spec.functions.len(), golden_spec.functions.len(), "Golden test function count mismatch");
            assert_eq!(spec.types.len(), golden_spec.types.len(), "Golden test type count mismatch");
            assert_eq!(spec, golden_spec, "Golden test extracted spec differs from fixture! Spec drift detected.");
        }
    }

    #[test]
    fn test_nested_and_tuple_types() {
        use stellar_xdr::curr::{ScSpecTypeTuple, ScSpecTypeVec, ScSpecTypeMap};

        let tuple_type = ScSpecTypeDef::Tuple(Box::new(ScSpecTypeTuple {
            value_types: vec![ScSpecTypeDef::U64, ScSpecTypeDef::Address].try_into().unwrap(),
        }));
        assert_eq!(type_def_to_string(&tuple_type), "(u64, Address)");

        let vec_type = ScSpecTypeDef::Vec(Box::new(ScSpecTypeVec {
            element_type: Box::new(tuple_type),
        }));
        assert_eq!(type_def_to_string(&vec_type), "Vec<(u64, Address)>");

        let map_type = ScSpecTypeDef::Map(Box::new(ScSpecTypeMap {
            key_type: Box::new(ScSpecTypeDef::Symbol),
            value_type: Box::new(ScSpecTypeDef::I128),
        }));
        assert_eq!(type_def_to_string(&map_type), "Map<Symbol, i128>");
    }
}
