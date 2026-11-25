//! Typed per-domain clients: one struct per domain, one async method per
//! command and one stream method (`on_<event>`) per event, all generic over
//! `chromiumoxide_types::CommandExecutor`.

use heck::{ToSnakeCase, ToUpperCamelCase};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use crate::pdl::{Domain, Protocol};

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "gen", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut",
    "pub", "ref", "return", "static", "struct", "super", "trait", "true", "type", "unsafe", "use",
    "where", "while", "yield",
];

fn method_ident(name: &str) -> Ident {
    if KEYWORDS.contains(&name) {
        format_ident!("r#{}", name)
    } else {
        format_ident!("{}", name)
    }
}

fn doc(description: Option<&str>, link: Option<String>) -> TokenStream {
    let text = match (description, link) {
        (Some(d), Some(link)) if !d.is_empty() => format!("{d}\n\n{link}"),
        (_, Some(link)) => link,
        (Some(d), None) => d.to_owned(),
        (None, None) => return TokenStream::new(),
    };
    quote! { #[doc = #text] }
}

/// Which items get a client method. Deprecated items never do (they stay
/// reachable through their raw command and event types); experimental ones do
/// when the type generator includes them.
pub(crate) struct Filter {
    pub experimental: bool,
    /// The docs' base URL, if items link to them.
    pub docs_url: Option<String>,
}

impl Filter {
    fn keep(&self, experimental: bool, deprecated: bool) -> bool {
        (self.experimental || !experimental) && !deprecated
    }
}

pub(crate) fn generate(
    protocols: &[Protocol],
    protocol_mods: &[String],
    filter: &Filter,
) -> TokenStream {
    let mut accessors = TokenStream::new();
    let mut clients = TokenStream::new();
    let mut domain_entries = Vec::new();
    let mut covered = Vec::new();

    let domains = protocols.iter().zip(protocol_mods).flat_map(|(p, m)| {
        p.domains
            .iter()
            .filter(|d| filter.keep(d.experimental, d.deprecated))
            .map(move |d| (d, m))
    });
    for (domain, protocol_mod) in domains {
        let (accessor, client) = generate_domain(domain, protocol_mod, filter, &mut covered);
        accessors.extend(accessor);
        clients.extend(client);
        let name = domain.name.as_ref();
        let snake = domain.name.to_snake_case();
        domain_entries.push(quote! { (#name, #snake) });
    }
    let covered = covered.iter().map(|(m, f)| quote! { (#m, #f) });

    quote! {
        /// Typed clients for every protocol domain.
        ///
        /// [`Protocol`] wraps anything that implements
        /// [`chromiumoxide_types::CommandExecutor`] (a page or a browser) and
        /// hands out one client per domain: `protocol.performance().get_metrics(..)`
        /// sends `Performance.getMetrics`, `protocol.network().on_request_will_be_sent()`
        /// streams `Network.requestWillBeSent`.
        #[allow(clippy::too_many_arguments)]
        pub mod domains {
            use chromiumoxide_types::CommandExecutor;

            /// `(protocol name, accessor)` of every domain with a client.
            pub const DOMAINS: &[(&str, &str)] = &[#(#domain_entries),*];

            /// `(Domain.method, client method)` of every command and event with a client method.
            pub const COVERED: &[(&str, &str)] = &[#(#covered),*];

            /// Every domain's client on one executor (a page or a browser).
            pub struct Protocol<'a, X: ?Sized> {
                exec: &'a X,
            }

            impl<X: ?Sized> Clone for Protocol<'_, X> {
                fn clone(&self) -> Self {
                    *self
                }
            }

            impl<X: ?Sized> Copy for Protocol<'_, X> {}

            impl<'a, X: ?Sized> Protocol<'a, X> {
                /// Wraps an executor.
                pub fn new(exec: &'a X) -> Self {
                    Self { exec }
                }
            }

            impl<'a, X: CommandExecutor + ?Sized> Protocol<'a, X> {
                #accessors
            }

            #clients
        }
    }
}

fn generate_domain(
    domain: &Domain,
    protocol_mod: &str,
    filter: &Filter,
    covered: &mut Vec<(String, String)>,
) -> (TokenStream, TokenStream) {
    let name = domain.name.as_ref();
    let accessor = method_ident(&name.to_snake_case());
    let client = format_ident!("{}", name.to_upper_camel_case());
    let protocol_mod = format_ident!("{}", protocol_mod);
    let module = format_ident!("{}", name.to_snake_case());
    let path = quote! { super::#protocol_mod::#module };
    let domain_doc = doc(
        domain.description.as_deref(),
        filter
            .docs_url
            .as_ref()
            .map(|u| format!("[{name}]({u}{name}/)")),
    );
    let accessor_doc = format!("The `{name}` domain.");

    let mut methods = TokenStream::new();
    for cmd in domain
        .commands
        .iter()
        .filter(|c| filter.keep(c.experimental, c.deprecated))
    {
        let cmd_name = cmd.name.as_ref();
        let method = cmd_name.to_snake_case();
        let ident = method_ident(&method);
        let params = format_ident!("{}Params", cmd_name.to_upper_camel_case());
        let returns = format_ident!("{}Returns", cmd_name.to_upper_camel_case());
        let doc = doc(
            cmd.description.as_deref(),
            Some(match &filter.docs_url {
                Some(u) => format!("Sends [`{name}.{cmd_name}`]({u}{name}/#method-{cmd_name})."),
                None => format!("Sends `{name}.{cmd_name}`."),
            }),
        );
        methods.extend(quote! {
            #doc
            pub async fn #ident(&self, params: #path::#params) -> Result<#path::#returns, X::Error> {
                self.exec.execute(params).await
            }
        });
        covered.push((format!("{name}.{cmd_name}"), method));
    }
    for ev in domain
        .events
        .iter()
        .filter(|e| filter.keep(e.experimental, e.deprecated))
    {
        let ev_name = ev.name.as_ref();
        let method = format!("on_{}", ev_name.to_snake_case());
        let ident = format_ident!("{}", method);
        let event = format_ident!("Event{}", ev_name.to_upper_camel_case());
        let doc = doc(
            ev.description.as_deref(),
            Some(match &filter.docs_url {
                Some(u) => {
                    format!("Streams [`{name}.{ev_name}`]({u}{name}/#event-{ev_name}) from now on.")
                }
                None => format!("Streams `{name}.{ev_name}` from now on."),
            }),
        );
        methods.extend(quote! {
            #doc
            pub fn #ident(&self) -> X::Events<#path::#event> {
                self.exec.events()
            }
        });
        covered.push((format!("{name}.{ev_name}"), method));
    }

    let accessor = quote! {
        #[doc = #accessor_doc]
        pub fn #accessor(&self) -> #client<'a, X> {
            #client { exec: self.exec }
        }
    };
    let client = quote! {
        #domain_doc
        pub struct #client<'a, X: ?Sized> {
            exec: &'a X,
        }

        impl<X: ?Sized> Clone for #client<'_, X> {
            fn clone(&self) -> Self {
                *self
            }
        }

        impl<X: ?Sized> Copy for #client<'_, X> {}

        impl<'a, X: CommandExecutor + ?Sized> #client<'a, X> {
            #methods
        }
    };
    (accessor, client)
}
