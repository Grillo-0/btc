extern crate proc_macro;

use proc_macro::{Delimiter, Group, Ident, Punct, Spacing, Span, TokenStream, TokenTree};

#[proc_macro_derive(BitcoinType)]
pub fn bitcoin_type_macro_derive(input: TokenStream) -> TokenStream {
    let mut input = input.into_iter();
    input.next();
    input.next();

    let type_name = input.next().unwrap();

    let atributes = if let TokenTree::Group(g) = input.next().unwrap() {
        let mut iter = g.stream().into_iter().peekable();
        let mut ret = vec![];

        while let Some(t) = iter.next() {
            if let Some(TokenTree::Punct(p)) = iter.peek() {
                if p.as_char() == ':' {
                    ret.push(t);
                }
            }
        }
        ret
    } else {
        panic!()
    };

    let tks: Vec<TokenTree> = vec![
        Ident::new("impl", Span::call_site()).into(),
        Ident::new("BitcoinType", Span::call_site()).into(),
        Ident::new("for", Span::call_site()).into(),
        type_name.clone(),
        Group::new(
            Delimiter::Brace,
            TokenStream::from_iter([gen_to_blob(&atributes), gen_from_blob(&atributes)].concat()),
        )
        .into(),
    ];

    TokenStream::from_iter(tks)
}

fn gen_func(
    name: &str,
    args: Vec<TokenTree>,
    body: Vec<TokenTree>,
    return_type: Vec<TokenTree>,
) -> Vec<TokenTree> {
    let mut ret = vec![
        Ident::new("fn", Span::call_site()).into(),
        Ident::new(name, Span::call_site()).into(),
        Group::new(Delimiter::Parenthesis, TokenStream::from_iter(args)).into(),
        Punct::new('-', Spacing::Joint).into(),
        Punct::new('>', Spacing::Alone).into(),
    ];
    ret.extend(return_type);
    ret.push(Group::new(Delimiter::Brace, TokenStream::from_iter(body)).into());
    ret
}

fn method_call(mut name: Vec<TokenTree>, method: &str, args: Vec<TokenTree>) -> Vec<TokenTree> {
    name.extend(vec![
        Punct::new('.', Spacing::Alone).into(),
        Ident::new(method, Span::call_site()).into(),
        Group::new(Delimiter::Parenthesis, TokenStream::from_iter(args)).into(),
    ]);
    name
}

fn gen_to_blob(atributes: &Vec<TokenTree>) -> Vec<TokenTree> {
    let args = vec![
        Punct::new('&', Spacing::Alone).into(),
        Ident::new("self", Span::call_site()).into(),
    ];

    let mut body: Vec<TokenTree> = vec![
        Ident::new("let", Span::call_site()).into(),
        Ident::new("mut", Span::call_site()).into(),
        Ident::new("ret", Span::call_site()).into(),
        Punct::new('=', Spacing::Alone).into(),
        Ident::new("vec", Span::call_site()).into(),
        Punct::new('!', Spacing::Alone).into(),
        Group::new(Delimiter::Bracket, TokenStream::new()).into(),
        Punct::new(';', Spacing::Alone).into(),
    ];

    for atrib in atributes {
        body.extend(method_call(
            vec![Ident::new("ret", Span::call_site()).into()],
            "extend",
            method_call(
                vec![
                    Ident::new("self", Span::call_site()).into(),
                    Punct::new('.', Spacing::Alone).into(),
                    atrib.clone(),
                ],
                "to_blob",
                vec![],
            ),
        ));
        body.push(Punct::new(';', Spacing::Alone).into());
    }
    body.push(Ident::new("ret", Span::call_site()).into());

    let ret = vec![
        Ident::new("Vec", Span::call_site()).into(),
        Punct::new('<', Spacing::Alone).into(),
        Ident::new("u8", Span::call_site()).into(),
        Punct::new('>', Spacing::Alone).into(),
    ];

    gen_func("to_blob", args, body, ret)
}

fn gen_from_blob(atributes: &Vec<TokenTree>) -> Vec<TokenTree> {
    let args = vec![
        Ident::new("blob", Span::call_site()).into(),
        Punct::new(':', Spacing::Alone).into(),
        Punct::new('&', Spacing::Alone).into(),
        Ident::new("mut", Span::call_site()).into(),
        Ident::new("Scanner", Span::call_site()).into(),
    ];

    let atribs = atributes.into_iter();
    let atribs = atribs.flat_map(|at| {
        [
            at.clone(),
            Punct::new(':', Spacing::Alone).into(),
            Ident::new("BitcoinType", Span::call_site()).into(),
            Punct::new(':', Spacing::Joint).into(),
            Punct::new(':', Spacing::Alone).into(),
            Ident::new("from_blob", Span::call_site()).into(),
            Group::new(
                Delimiter::Parenthesis,
                TokenStream::from_iter(Vec::<TokenTree>::from([Ident::new(
                    "blob",
                    Span::call_site(),
                )
                .into()])),
            )
            .into(),
            Punct::new(',', Spacing::Alone).into(),
        ]
    });

    let body: Vec<TokenTree> = vec![
        Ident::new("Self", Span::call_site()).into(),
        Group::new(Delimiter::Brace, TokenStream::from_iter(atribs)).into(),
    ];

    gen_func(
        "from_blob",
        args,
        body,
        vec![Ident::new("Self", Span::call_site()).into()],
    )
}
