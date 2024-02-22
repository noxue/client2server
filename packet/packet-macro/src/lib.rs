extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{self, DeriveInput};

#[proc_macro_derive(Pack)]
pub fn pack_macro_derive(input: TokenStream) -> TokenStream {
    // Construct a representation of Rust code as a syntax tree
    // that we can manipulate
    let ast = syn::parse(input).unwrap();

    // Build the trait implementation
    impl_pack_macro(&ast)
}

fn impl_pack_macro(ast: &DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let generics = &ast.generics;

    // 检查是否有泛型参数
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let gen = if generics.params.is_empty() {
        // 没有泛型参数的情况
        quote! {
            impl Pack for #name {
                fn pack(&self) -> Result<Vec<u8>, String> {
                    pack(self)
                }
            }
        }
    } else {
        // 有泛型参数的情况
        quote! {
            impl #impl_generics Pack for #name #ty_generics where
            T: Serialize + PartialEq + Debug + Default, {
                fn pack(&self) -> Result<Vec<u8>, String> {
                    pack(self)
                }
            }
        }
    };

    gen.into()
}

#[proc_macro_derive(UnPack)]
pub fn unpack_macro_derive(input: TokenStream) -> TokenStream {
    // Construct a representation of Rust code as a syntax tree
    // that we can manipulate
    let ast = syn::parse(input).unwrap();

    // Build the trait implementation
    impl_unpack_macro(&ast)
}

fn impl_unpack_macro(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let generics = &ast.generics;

    // 检查是否有泛型参数
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let gen = if generics.params.is_empty() {
        // 没有泛型参数的情况
        quote! {
            impl UnPack for #name {
                fn unpack(encoded: &[u8]) -> Result<Self, String> {
                    unpack(encoded)
                }
            }
        }
    } else {
        // 有泛型参数的情况
        quote! {
            impl #impl_generics UnPack for #name #ty_generics where
            T: DeserializeOwned + PartialEq + Debug+ Serialize+Default,{
                fn unpack(encoded: &[u8]) -> Result<Self, String> {
                    unpack(encoded)
                }
            }
        }
    };

    gen.into()
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
