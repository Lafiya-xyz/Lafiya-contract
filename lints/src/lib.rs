#![feature(rustc_private)]
#![warn(unused_extern_crates)]

extern crate rustc_hir;
extern crate rustc_lint;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

use clippy_utils::diagnostics::span_lint;
use rustc_hir::intravisit::{walk_expr, FnKind, Visitor};
use rustc_hir::{BinOpKind, Body, Expr, ExprKind, FnDecl};
use rustc_lint::{LateContext, LateLintPass};
use rustc_middle::ty::TyKind;
use rustc_span::{def_id::LocalDefId, Span};

dylint_linting::dylint_library!();

#[unsafe(no_mangle)]
pub fn register_lints(sess: &rustc_session::Session, store: &mut rustc_lint::LintStore) {
    dylint_linting::init_config(sess);
    store.register_lints(&[PERSISTENT_SET_WITHOUT_TTL, STORAGE_IN_LOOP, UNCHECKED_AMOUNT_ARITHMETIC]);
    store.register_late_lint_pass(Box::new(|_| Box::new(LafiyaLints)));
}

rustc_session::declare_lint! {
    /// Persistent `set` with no `extend_ttl` in the same function.
    pub PERSISTENT_SET_WITHOUT_TTL, Warn, "persistent storage write without a nearby extend_ttl"
}
rustc_session::declare_lint! {
    /// Storage access inside a loop.
    pub STORAGE_IN_LOOP, Warn, "storage access inside a loop"
}
rustc_session::declare_lint! {
    /// `+ - *` on an i128/u128 amount.
    pub UNCHECKED_AMOUNT_ARITHMETIC, Warn, "unchecked arithmetic on an amount type"
}
rustc_session::declare_lint_pass!(LafiyaLints => [PERSISTENT_SET_WITHOUT_TTL, STORAGE_IN_LOOP, UNCHECKED_AMOUNT_ARITHMETIC]);

/// `x.storage().persistent().set(..)` etc: returns the storage kind and method.
fn storage_call<'a>(e: &'a Expr<'_>) -> Option<(&'a str, &'a str)> {
    let ExprKind::MethodCall(seg, recv, _, _) = e.kind else { return None };
    let ExprKind::MethodCall(kind, inner, _, _) = recv.kind else { return None };
    let ExprKind::MethodCall(storage, _, _, _) = inner.kind else { return None };
    if storage.ident.name.as_str() != "storage" {
        return None;
    }
    Some((kind.ident.name.as_str(), seg.ident.name.as_str()))
}

struct Scan<'a, 'tcx> {
    cx: &'a LateContext<'tcx>,
    loop_depth: u32,
    persistent_sets: Vec<Span>,
    has_extend_ttl: bool,
}

impl<'tcx> Visitor<'tcx> for Scan<'_, 'tcx> {
    fn visit_expr(&mut self, e: &'tcx Expr<'tcx>) {
        if let Some((kind, method)) = storage_call(e) {
            if self.loop_depth > 0 {
                span_lint(self.cx, STORAGE_IN_LOOP, e.span, "storage access inside a loop; hoist or batch it");
            }
            if kind == "persistent" && method == "set" {
                self.persistent_sets.push(e.span);
            }
            if method == "extend_ttl" {
                self.has_extend_ttl = true;
            }
        }
        if let ExprKind::Binary(op, l, _) = e.kind {
            if matches!(op.node, BinOpKind::Add | BinOpKind::Sub | BinOpKind::Mul)
                && matches!(self.cx.typeck_results().expr_ty(l).kind(), TyKind::Int(rustc_middle::ty::IntTy::I128) | TyKind::Uint(rustc_middle::ty::UintTy::U128))
            {
                span_lint(self.cx, UNCHECKED_AMOUNT_ARITHMETIC, e.span, "use checked_add/checked_sub/checked_mul on amounts");
            }
        }
        if matches!(e.kind, ExprKind::Loop(..)) {
            self.loop_depth += 1;
            walk_expr(self, e);
            self.loop_depth -= 1;
        } else {
            walk_expr(self, e);
        }
    }
}

impl<'tcx> LateLintPass<'tcx> for LafiyaLints {
    fn check_fn(&mut self, cx: &LateContext<'tcx>, _: FnKind<'tcx>, _: &'tcx FnDecl<'tcx>, body: &'tcx Body<'tcx>, _: Span, _: LocalDefId) {
        let mut scan = Scan { cx, loop_depth: 0, persistent_sets: vec![], has_extend_ttl: false };
        scan.visit_expr(body.value);
        if !scan.has_extend_ttl {
            for span in scan.persistent_sets {
                span_lint(cx, PERSISTENT_SET_WITHOUT_TTL, span, "persistent set without extend_ttl in the same function");
            }
        }
    }
}

#[test]
fn ui() {
    dylint_testing::ui_test(env!("CARGO_PKG_NAME"), "ui");
}
