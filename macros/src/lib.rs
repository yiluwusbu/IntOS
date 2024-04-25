use proc_macro::TokenStream;
use quote::quote;
use syn::ItemFn;

#[proc_macro_attribute]
pub fn app(args: TokenStream, item: TokenStream) -> TokenStream {
    task_fn_instrument(args, item)
}

#[proc_macro_attribute]
pub fn task(args: TokenStream, item: TokenStream) -> TokenStream {
    task_fn_instrument(args, item)
}

#[proc_macro_attribute]
pub fn idempotent(args: TokenStream, item: TokenStream) -> TokenStream {
    task_fn_instrument(args, item)
}

fn task_fn_instrument(_args: TokenStream, item: TokenStream) -> TokenStream {
    let item = syn::parse::<ItemFn>(item).unwrap();
    let sig = item.sig;
    let vis = item.vis;
    let args = &sig.inputs;
    assert!(
        args.len() <= 1,
        "Application task can't take more than 1 parameter"
    );
    let block = item.block;
    quote!(
        #[allow(dead_code)]
        #vis #sig {
            #block;
            loop {

            }
        }
    )
    .into()
}
