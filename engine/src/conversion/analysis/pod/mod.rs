// Copyright 2020 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

mod byvalue_checker;

use std::collections::HashSet;

use autocxx_parser::IncludeCppConfig;
use byvalue_checker::ByValueChecker;
use syn::{ItemEnum, ItemStruct, Type};

use crate::{
    conversion::{
        analysis::type_converter::{add_analysis, TypeConversionContext, TypeConverter},
        api::{AnalysisPhase, Api, ApiName, TypeKind, UnanalyzedApi},
        convert_error::{ConvertErrorWithContext, ErrorContext},
        error_reporter::convert_apis,
        ConvertError,
    },
    types::{Namespace, QualifiedName},
};

use super::tdef::{TypedefAnalysis, TypedefPhase};

pub(crate) struct PodAnalysis {
    pub(crate) kind: TypeKind,
    pub(crate) bases: HashSet<QualifiedName>,
    pub(crate) field_deps: HashSet<QualifiedName>,
}

pub(crate) struct PodPhase;

impl AnalysisPhase for PodPhase {
    type TypedefAnalysis = TypedefAnalysis;
    type StructAnalysis = PodAnalysis;
    type FunAnalysis = ();
}

/// In our set of APIs, work out which ones are safe to represent
/// by value in Rust (e.g. they don't have a destructor) and record
/// as such. Return a set of APIs annotated with extra metadata,
/// and an object which can be used to query the POD status of any
/// type whether or not it's one of the [Api]s.
pub(crate) fn analyze_pod_apis(
    apis: Vec<Api<TypedefPhase>>,
    config: &IncludeCppConfig,
) -> Result<Vec<Api<PodPhase>>, ConvertError> {
    // This next line will return an error if any of the 'generate_pod'
    // directives from the user can't be met because, for instance,
    // a type contains a std::string or some other type which can't be
    // held safely by value in Rust.
    let byvalue_checker = ByValueChecker::new_from_apis(&apis, config)?;
    let mut extra_apis = Vec::new();
    let mut type_converter = TypeConverter::new(config, &apis);
    let mut results = Vec::new();
    convert_apis(
        apis,
        &mut results,
        Api::fun_unchanged,
        |name, item, _| {
            analyze_struct(
                &byvalue_checker,
                &mut type_converter,
                &mut extra_apis,
                name,
                item,
            )
        },
        analyze_enum,
        Api::typedef_unchanged,
    );
    // Conceivably, the process of POD-analysing the first set of APIs could result
    // in us creating new APIs to concretize generic types.
    let extra_apis: Vec<Api<PodPhase>> = extra_apis.into_iter().map(add_analysis).collect();
    let mut more_extra_apis = Vec::new();
    convert_apis(
        extra_apis,
        &mut results,
        Api::fun_unchanged,
        |name, item, _| {
            analyze_struct(
                &byvalue_checker,
                &mut type_converter,
                &mut more_extra_apis,
                name,
                item,
            )
        },
        analyze_enum,
        Api::typedef_unchanged,
    );
    assert!(more_extra_apis.is_empty());
    Ok(results)
}

fn analyze_enum(
    name: ApiName,
    mut item: ItemEnum,
) -> Result<Box<dyn Iterator<Item = Api<PodPhase>>>, ConvertErrorWithContext> {
    super::remove_bindgen_attrs(&mut item.attrs, name.name.get_final_ident())?;
    Ok(Box::new(std::iter::once(Api::Enum { name, item })))
}

fn analyze_struct(
    byvalue_checker: &ByValueChecker,
    type_converter: &mut TypeConverter,
    extra_apis: &mut Vec<UnanalyzedApi>,
    name: ApiName,
    mut item: ItemStruct,
) -> Result<Box<dyn Iterator<Item = Api<PodPhase>>>, ConvertErrorWithContext> {
    let id = name.name.get_final_ident();
    super::remove_bindgen_attrs(&mut item.attrs, id.clone())?;
    let bases = get_bases(&item);
    let mut field_deps = HashSet::new();
    let type_kind = if byvalue_checker.is_pod(&name.name) {
        // It's POD so let's mark dependencies on things in its field
        get_struct_field_types(
            type_converter,
            name.name.get_namespace(),
            &item,
            &mut field_deps,
            extra_apis,
        )
        .map_err(|e| ConvertErrorWithContext(e, Some(ErrorContext::Item(id))))?;
        TypeKind::Pod
    } else if name.is_nested_struct_or_class() {
        TypeKind::NonPodNested
    } else {
        TypeKind::NonPod
    };
    Ok(Box::new(std::iter::once(Api::Struct {
        name,
        item,
        analysis: PodAnalysis {
            kind: type_kind,
            bases,
            field_deps,
        },
    })))
}

fn get_struct_field_types(
    type_converter: &mut TypeConverter,
    ns: &Namespace,
    s: &ItemStruct,
    deps: &mut HashSet<QualifiedName>,
    extra_apis: &mut Vec<UnanalyzedApi>,
) -> Result<(), ConvertError> {
    for f in &s.fields {
        let annotated =
            type_converter.convert_type(f.ty.clone(), ns, &TypeConversionContext::CxxInnerType)?;
        extra_apis.extend(annotated.extra_apis);
        deps.extend(annotated.types_encountered);
    }
    Ok(())
}

fn get_bases(item: &ItemStruct) -> HashSet<QualifiedName> {
    item.fields
        .iter()
        .filter_map(|f| match &f.ty {
            Type::Path(typ) => f
                .ident
                .as_ref()
                .filter(|id| id.to_string().starts_with("_base"))
                .map(|_| QualifiedName::from_type_path(typ)),
            _ => None,
        })
        .collect()
}
