// Copyright 2021 Google LLC
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

use std::collections::HashSet;

use itertools::Itertools;
use syn::{
    punctuated::Punctuated, Binding, Expr, ExprAssign, ExprAssignOp, ExprAwait, ExprBinary,
    ExprBox, ExprBreak, ExprCast, ExprField, ExprGroup, ExprLet, ExprParen, ExprReference, ExprTry,
    ExprType, ExprUnary, ImplItem, Item, Pat, PatBox, PatReference, PatSlice, PatTuple, Path,
    ReturnType, Stmt, TraitItem, Type, TypeArray, TypeGroup, TypeParamBound, TypeParen, TypePtr,
    TypeReference, TypeSlice,
};

#[derive(Default)]
pub(super) struct CppList(pub(super) HashSet<String>);

impl CppList {
    pub(super) fn search_item(&mut self, item: &Item) {
        match item {
            Item::Fn(fun) => {
                for stmt in &fun.block.stmts {
                    self.search_stmt(stmt)
                }
                self.search_return_type(&fun.sig.output);
                for i in &fun.sig.inputs {
                    match i {
                        syn::FnArg::Receiver(_) => {}
                        syn::FnArg::Typed(pt) => {
                            self.search_pat(&pt.pat);
                            self.search_type(&pt.ty);
                        }
                    }
                }
            }
            Item::Impl(imp) => {
                for item in &imp.items {
                    self.search_impl_item(item)
                }
            }
            Item::Mod(md) => {
                if let Some((_, items)) = &md.content {
                    for item in items {
                        self.search_item(item)
                    }
                }
            }
            Item::Trait(tr) => {
                for item in &tr.items {
                    self.search_trait_item(item)
                }
            }
            _ => {}
        }
    }

    fn search_path(&mut self, path: &Path) {
        let mut seg_iter = path.segments.iter();
        if let Some(first_seg) = seg_iter.next() {
            if first_seg.ident == "ffi" {
                self.0
                    .insert(seg_iter.map(|seg| seg.ident.to_string()).join("::"));
            }
        }
        for seg in path.segments.iter() {
            self.search_path_arguments(&seg.arguments);
        }
    }

    fn search_trait_item(&mut self, itm: &TraitItem) {
        if let TraitItem::Method(itm) = itm {
            if let Some(block) = &itm.default {
                self.search_stmts(block.stmts.iter())
            }
        }
    }

    fn search_stmts<'a>(&mut self, stmts: impl Iterator<Item = &'a Stmt>) {
        for stmt in stmts {
            self.search_stmt(stmt)
        }
    }

    fn search_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Local(lcl) => {
                if let Some((_, expr)) = &lcl.init {
                    self.search_expr(expr)
                }
                self.search_pat(&lcl.pat);
            }
            Stmt::Item(itm) => self.search_item(itm),
            Stmt::Expr(exp) | Stmt::Semi(exp, _) => self.search_expr(exp),
        }
    }

    fn search_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Path(exp) => {
                self.search_path(&exp.path);
            }
            Expr::Macro(_) => {}
            Expr::Array(array) => self.search_exprs(array.elems.iter()),
            Expr::Assign(ExprAssign { left, right, .. })
            | Expr::AssignOp(ExprAssignOp { left, right, .. })
            | Expr::Binary(ExprBinary { left, right, .. }) => {
                self.search_expr(left);
                self.search_expr(right);
            }
            Expr::Async(ass) => self.search_stmts(ass.block.stmts.iter()),
            Expr::Await(ExprAwait { base, .. }) | Expr::Field(ExprField { base, .. }) => {
                self.search_expr(base)
            }
            Expr::Block(blck) => self.search_stmts(blck.block.stmts.iter()),
            Expr::Box(ExprBox { expr, .. })
            | Expr::Break(ExprBreak {
                expr: Some(expr), ..
            })
            | Expr::Cast(ExprCast { expr, .. })
            | Expr::Group(ExprGroup { expr, .. })
            | Expr::Paren(ExprParen { expr, .. })
            | Expr::Reference(ExprReference { expr, .. })
            | Expr::Try(ExprTry { expr, .. })
            | Expr::Type(ExprType { expr, .. })
            | Expr::Unary(ExprUnary { expr, .. }) => self.search_expr(expr),
            Expr::Call(exc) => {
                self.search_expr(&exc.func);
                self.search_exprs(exc.args.iter());
            }
            Expr::Closure(cls) => self.search_expr(&cls.body),
            Expr::Continue(_)
            | Expr::Lit(_)
            | Expr::Break(ExprBreak { expr: None, .. })
            | Expr::Verbatim(_) => {}
            Expr::ForLoop(fl) => {
                self.search_expr(&fl.expr);
                self.search_stmts(fl.body.stmts.iter());
            }
            Expr::If(exif) => {
                self.search_expr(&exif.cond);
                self.search_stmts(exif.then_branch.stmts.iter());
                if let Some((_, else_branch)) = &exif.else_branch {
                    self.search_expr(else_branch);
                }
            }
            Expr::Index(exidx) => {
                self.search_expr(&exidx.expr);
                self.search_expr(&exidx.index);
            }
            Expr::Let(ExprLet { expr, pat, .. }) => {
                self.search_expr(expr);
                self.search_pat(pat);
            }
            Expr::Loop(exloo) => self.search_stmts(exloo.body.stmts.iter()),
            Expr::Match(exm) => {
                self.search_expr(&exm.expr);
                for a in &exm.arms {
                    self.search_expr(&a.body);
                    if let Some((_, guard)) = &a.guard {
                        self.search_expr(guard);
                    }
                }
            }
            Expr::MethodCall(mtc) => {
                self.search_expr(&mtc.receiver);
                self.search_exprs(mtc.args.iter());
            }
            Expr::Range(exr) => {
                self.search_option_expr(&exr.from);
                self.search_option_expr(&exr.to);
            }
            Expr::Repeat(exr) => {
                self.search_expr(&exr.expr);
                self.search_expr(&exr.len);
            }
            Expr::Return(exret) => {
                if let Some(expr) = &exret.expr {
                    self.search_expr(expr);
                }
            }
            Expr::Struct(exst) => {
                for f in &exst.fields {
                    self.search_expr(&f.expr);
                }
                self.search_option_expr(&exst.rest);
            }
            Expr::TryBlock(extb) => self.search_stmts(extb.block.stmts.iter()),
            Expr::Tuple(ext) => self.search_exprs(ext.elems.iter()),
            Expr::Unsafe(exs) => self.search_stmts(exs.block.stmts.iter()),
            Expr::While(exw) => {
                self.search_expr(&exw.cond);
                self.search_stmts(exw.body.stmts.iter());
            }
            Expr::Yield(exy) => self.search_option_expr(&exy.expr),
            Expr::__TestExhaustive(_) => {}
        }
    }

    fn search_option_expr(&mut self, expr: &Option<Box<Expr>>) {
        if let Some(expr) = &expr {
            self.search_expr(expr);
        }
    }

    fn search_exprs<'a>(&mut self, exprs: impl Iterator<Item = &'a Expr>) {
        for e in exprs {
            self.search_expr(e);
        }
    }

    fn search_impl_item(&mut self, impl_item: &ImplItem) {
        if let ImplItem::Method(itm) = impl_item {
            for stmt in &itm.block.stmts {
                self.search_stmt(stmt)
            }
        }
    }

    fn search_pat(&mut self, pat: &Pat) {
        match pat {
            Pat::Box(PatBox { pat, .. }) | Pat::Reference(PatReference { pat, .. }) => {
                self.search_pat(pat)
            }
            Pat::Ident(_) | Pat::Lit(_) | Pat::Macro(_) | Pat::Range(_) | Pat::Rest(_) => {}
            Pat::Or(pator) => {
                for case in &pator.cases {
                    self.search_pat(case);
                }
            }
            Pat::Path(pp) => self.search_path(&pp.path),
            Pat::Slice(PatSlice { elems, .. }) | Pat::Tuple(PatTuple { elems, .. }) => {
                for case in elems {
                    self.search_pat(case);
                }
            }
            Pat::Struct(ps) => {
                self.search_path(&ps.path);
                for f in &ps.fields {
                    self.search_pat(&f.pat);
                }
            }
            Pat::TupleStruct(tps) => {
                self.search_path(&tps.path);
                for f in &tps.pat.elems {
                    self.search_pat(f);
                }
            }
            Pat::Type(pt) => {
                self.search_pat(&pt.pat);
                self.search_type(&pt.ty);
            }
            _ => {}
        }
    }

    fn search_type(&mut self, ty: &Type) {
        match ty {
            Type::Array(TypeArray { elem, .. })
            | Type::Group(TypeGroup { elem, .. })
            | Type::Paren(TypeParen { elem, .. })
            | Type::Ptr(TypePtr { elem, .. })
            | Type::Reference(TypeReference { elem, .. })
            | Type::Slice(TypeSlice { elem, .. }) => self.search_type(elem),
            Type::BareFn(tf) => {
                for input in &tf.inputs {
                    self.search_type(&input.ty);
                }
                self.search_return_type(&tf.output);
            }
            Type::ImplTrait(tyit) => {
                for b in &tyit.bounds {
                    if let syn::TypeParamBound::Trait(tyt) = b {
                        self.search_path(&tyt.path)
                    }
                }
            }
            Type::Infer(_) | Type::Macro(_) | Type::Never(_) => {}
            Type::Path(typ) => self.search_path(&typ.path),
            Type::TraitObject(tto) => self.search_type_param_bounds(&tto.bounds),
            Type::Tuple(tt) => {
                for e in &tt.elems {
                    self.search_type(e)
                }
            }
            _ => {}
        }
    }

    fn search_type_param_bounds(&mut self, bounds: &Punctuated<TypeParamBound, syn::token::Add>) {
        for b in bounds {
            if let syn::TypeParamBound::Trait(tpbt) = b {
                self.search_path(&tpbt.path)
            }
        }
    }

    fn search_return_type(&mut self, output: &ReturnType) {
        if let ReturnType::Type(_, ty) = &output {
            self.search_type(ty)
        }
    }

    fn search_path_arguments(&mut self, arguments: &syn::PathArguments) {
        match arguments {
            syn::PathArguments::None => {}
            syn::PathArguments::AngleBracketed(paab) => {
                for arg in &paab.args {
                    match arg {
                        syn::GenericArgument::Lifetime(_) => {}
                        syn::GenericArgument::Type(ty)
                        | syn::GenericArgument::Binding(Binding { ty, .. }) => self.search_type(ty),
                        syn::GenericArgument::Constraint(c) => {
                            self.search_type_param_bounds(&c.bounds)
                        }
                        syn::GenericArgument::Const(c) => self.search_expr(c),
                    }
                }
            }
            syn::PathArguments::Parenthesized(pas) => {
                self.search_return_type(&pas.output);
                for t in &pas.inputs {
                    self.search_type(t);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use syn::{parse_quote, Item};

    use super::CppList;

    fn assert_found(cpp_list: &CppList) {
        assert!(!cpp_list.0.is_empty());
        assert!(cpp_list.0.iter().next().unwrap() == "xxx");
    }

    #[test]
    fn test_mod_plain_call() {
        let mut cpplist = CppList::default();
        let itm = Item::Mod(parse_quote! {
            mod foo {
                fn bar() {
                    ffi::xxx()
                }
            }
        });
        cpplist.search_item(&itm);
        assert_found(&cpplist);
    }

    #[test]
    fn test_plain_call() {
        let mut cpplist = CppList::default();
        let itm = Item::Fn(parse_quote! {
            fn bar() {
                ffi::xxx()
            }
        });
        cpplist.search_item(&itm);
        assert_found(&cpplist);
    }

    #[test]
    fn test_plain_call_with_semi() {
        let mut cpplist = CppList::default();
        let itm = Item::Fn(parse_quote! {
            fn bar() {
                ffi::xxx();
            }
        });
        cpplist.search_item(&itm);
        assert_found(&cpplist);
    }

    #[test]
    fn test_in_ns() {
        let mut cpplist = CppList::default();
        let itm = Item::Fn(parse_quote! {
            fn bar() {
                ffi::a::b::xxx();
            }
        });
        cpplist.search_item(&itm);
        assert!(!cpplist.0.is_empty());
        assert!(cpplist.0.iter().next().unwrap() == "a::b::xxx");
    }

    #[test]
    fn test_deep_nested_thingy() {
        let mut cpplist = CppList::default();
        let itm = Item::Fn(parse_quote! {
            fn bar() {
                a + 3 * foo(ffi::xxx());
            }
        });
        cpplist.search_item(&itm);
        assert_found(&cpplist);
    }

    #[test]
    fn test_ty_in_let() {
        let mut cpplist = CppList::default();
        let itm = Item::Fn(parse_quote! {
            fn bar() {
                let foo: ffi::xxx = bar();
            }
        });
        cpplist.search_item(&itm);
        assert_found(&cpplist);
    }

    #[test]
    fn test_ty_in_fn() {
        let mut cpplist = CppList::default();
        let itm = Item::Fn(parse_quote! {
            fn bar(a: &mut ffi::xxx) {
            }
        });
        cpplist.search_item(&itm);
        assert_found(&cpplist);
    }

    #[test]
    fn test_ty_in_fn_up() {
        let mut cpplist = CppList::default();
        let itm = Item::Fn(parse_quote! {
            fn bar(a: cxx::UniquePtr<ffi::xxx>) {
            }
        });
        cpplist.search_item(&itm);
        assert_found(&cpplist);
    }
}
