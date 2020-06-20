use proc_macro::TokenStream;
use syn;
use syn::export::Span;

use crate::config::Config;
use crate::meta;
use crate::meta::attr::{MetaAttr, MetaAttrType};
use crate::parsertree::ParserTree;
use crate::structs::{get_pre_post_exec, parse_fields, StructParserTree};

#[derive(Debug)]
struct VariantParserTree{
    pub ident: syn::Ident,
    pub selector: String,
    pub struct_def: StructParserTree,
}

fn parse_variant(variant: &syn::Variant, config: &Config) -> VariantParserTree {
    // eprintln!("variant: {:?}", variant);
    let meta_list = meta::parse_nom_attribute(&variant.attrs).expect("Parsing the 'nom' meta attribute failed");
    let selector = get_selector(&meta_list).expect(&format!("The 'Selector' attribute must be used to give the value of selector item (variant {})", variant.ident));
    let struct_def = parse_fields(&variant.fields, config);
    // discriminant ?
    VariantParserTree{
        ident: variant.ident.clone(),
        selector,
        struct_def
    }
}

fn get_selector(meta_list: &[MetaAttr]) -> Option<String> {
    for meta in meta_list {
        match meta.attr_type {
            MetaAttrType::Selector => {
                return Some(meta.arg().unwrap().to_string().clone());
            }
            _ => (),
        }
    }
    None
}

fn get_repr(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if let Ok(ref meta) = attr.parse_meta() {
            match meta {
                syn::Meta::NameValue(_) => (),
                syn::Meta::List(ref metalist) => {
                    if let Some(ident) = metalist.path.get_ident() {
                        if &ident == &"repr" {
                            for n in metalist.nested.iter() {
                                match n {
                                    syn::NestedMeta::Meta(meta) => {
                                        match meta {
                                            syn::Meta::Path(path) => {
                                                if let Some(word) = path.get_ident() {
                                                    return Some(word.to_string())
                                                } else {
                                                    panic!("unsupported nested type for 'repr'")
                                                }
                                            },
                                            _ => panic!("unsupported nested type for 'repr'")
                                        }
                                    },
                                    _ => panic!("unsupported meta type for 'repr'")
                                }
                            }
                        }
                    }
                }
                syn::Meta::Path(_) => ()
            }
        }
    }
    None
}

fn is_input_fieldless_enum(ast: &syn::DeriveInput) -> bool {
    match ast.data {
        syn::Data::Enum(ref data_enum) => {
            // eprintln!("{:?}", data_enum);
            data_enum.variants.iter()
                .fold(true,
                      |acc, v| {
                          if let syn::Fields::Unit = v.fields { acc } else { false }
                      })
        },
        _ => false
    }
}

fn impl_nom_fieldless_enums(ast: &syn::DeriveInput, repr:String, meta_list: &[MetaAttr], config: &Config) -> TokenStream {
    let input_name = syn::Ident::new(&config.input_name, Span::call_site());
    let orig_input_name = syn::Ident::new(&("orig_".to_string() + &config.input_name), Span::call_site());
    let (tl_pre, _tl_post) = get_pre_post_exec(&meta_list, config);
    let parser = match repr.as_ref() {
        "u8"  |
        "u16" |
        "u24" |
        "u32" |
        "u64" |
        "i8"  |
        "i16" |
        "i24" |
        "i32" |
        "i64" => {
            let is_big_endian = if meta_list.iter().any(|m| m.is_type(MetaAttrType::BigEndian)) {
                true
            } else if meta_list.iter().any(|m| m.is_type(MetaAttrType::LittleEndian)) {
                false
            } else {
                config.big_endian
            };
            if is_big_endian {
                Some(ParserTree::Raw(format!("nom::number::streaming::be_{}", repr)))
            } else {
                Some(ParserTree::Raw(format!("nom::number::streaming::le_{}", repr)))
            }
        }
        _ => panic!("Cannot parse 'repr' content")
    };
    let variant_names : Vec<_> =
        match ast.data {
            syn::Data::Enum(ref data_enum) => {
                // eprintln!("{:?}", data_enum);
                data_enum.variants.iter()
                    .map(|v| {
                        v.ident.to_string()
                    })
                    .collect()
            },
            _ => { panic!("expect enum"); }
        };
    let generics = &ast.generics;
    let name = &ast.ident;
    let ty = syn::Ident::new(&repr, Span::call_site());
    let variants_code : Vec<_> =
        variant_names.iter()
            .map(|variant_name| {
                let id = syn::Ident::new(variant_name, Span::call_site());
                quote!{ if selector == #name::#id as #ty { return Ok((#input_name, #name::#id)); } }
            })
            .collect();
    let tokens = quote!{
        impl#generics #name#generics {
            fn parse(#orig_input_name: &[u8]) -> nom::IResult<&[u8],#name> {
                let #input_name = #orig_input_name;
                #tl_pre
                let (#input_name, selector) = #parser(#input_name)?;
                #(#variants_code)*
                Err(::nom::Err::Error((#orig_input_name, ::nom::error::ErrorKind::Switch)))
            }
        }
    };
    if config.debug_derive {
        eprintln!("impl_nom_enums: {}", tokens);
    }

    tokens.into()
}

pub(crate) fn impl_nom_enums(ast: &syn::DeriveInput, config: &Config) -> TokenStream {
    let name = &ast.ident;
    // eprintln!("{:?}", ast.attrs);
    let meta_list = meta::parse_nom_top_level_attribute(&ast.attrs).expect("Parsing the 'nom' meta attribute failed");
    let input_name = syn::Ident::new(&config.input_name, Span::call_site());
    let orig_input_name = syn::Ident::new(&("orig_".to_string() + &config.input_name), Span::call_site());
    let selector = match get_selector(&meta_list) { //.expect("The 'Selector' attribute must be used to give the type of selector item");
        Some(s) => s,
        None    => {
            if is_input_fieldless_enum(ast) {
                // check that we have a repr attribute
                let repr = get_repr(&ast.attrs).expect("Nom-derive: fieldless enums must have a 'repr' attribute");
                return impl_nom_fieldless_enums(ast, repr, &meta_list, config);
            } else {
                panic!("Nom-derive: enums must specify the 'selector' attribute");
            }
        }
    };
    let mut variants_defs : Vec<_> =
        match ast.data {
            syn::Data::Enum(ref data_enum) => {
                // eprintln!("{:?}", data_enum);
                data_enum.variants.iter()
                    .map(|v| parse_variant(v, config))
                    .collect()
            },
            _ => { panic!("expect enum"); }
        };
    // parse string items and prepare tokens for each variant
    let (tl_pre, _tl_post) = get_pre_post_exec(&meta_list, config);
    let generics = &ast.generics;
    let selector_type : proc_macro2::TokenStream = selector.parse().unwrap();
    let mut default_case_handled = false;
    let mut variants_code : Vec<_> = {
        variants_defs.iter()
            .map(|def| {
                if def.selector == "_" { default_case_handled = true; }
                let m : proc_macro2::TokenStream = def.selector.parse().expect("invalid selector value");
                let variantname = &def.ident;
                let (idents,parser_tokens) : (Vec<_>,Vec<_>) = def.struct_def.parsers.iter()
                    .map(|sp| {
                        let id = syn::Ident::new(&sp.name, Span::call_site());
                        (id, &sp.parser)
                    })
                    .unzip();
                let idents2 = idents.clone();
                let struct_def = match def.struct_def.unnamed {
                    false => quote!{ ( #name::#variantname { #(#idents2),* } ) },
                    true  => quote!{ ( #name::#variantname ( #(#idents2),* ) ) },
                };
                quote!{
                    #m => {
                        #(let (#input_name, #idents) = #parser_tokens (#input_name) ?;)*
                        let struct_def = #struct_def;
                        Ok((#input_name, struct_def))
                        // Err(nom::Err::Error(error_position!(#input_name, nom::ErrorKind::Switch)))
                    },
                }
            })
            .collect()
    };
    // if we have a default case, make sure it is the last entry
    if default_case_handled {
        let pos = variants_defs.iter()
            .position(|def| def.selector == "_")
            .expect("default case is handled but couldn't find index");
        let last_index = variants_defs.len() - 1;
        if pos != last_index {
            variants_defs.swap(pos, last_index);
            variants_code.swap(pos, last_index);
        }
    }
    // generate code
    let default_case =
        if default_case_handled { quote!{} }
        else { quote!{ _ => Err(nom::Err::Error(nom::error_position!(#input_name, nom::error::ErrorKind::Switch))) } };
    let tokens = quote!{
        impl#generics #name#generics {
            fn parse(#orig_input_name: &[u8], selector: #selector_type) -> nom::IResult<&[u8],#name> {
                let #input_name = #orig_input_name;
                #tl_pre
                let enum_def = match selector {
                    #(#variants_code)*
                    #default_case
                };
                enum_def
            }
        }
    };

    if config.debug_derive {
        eprintln!("impl_nom_enums: {}", tokens);
    }

    tokens.into()
}
