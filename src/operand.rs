use rustc_abi::Size;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::interpret::{AllocRange, ConstAllocation, GlobalAlloc, Pointer, Scalar};
use rustc_middle::mir::{Const, ConstValue, Local, Operand, Place};
use rustc_middle::ty::ScalarInt;
use rustc_middle::ty::{ParamEnv, Ty, TyCtxt, TyKind};

// Get the DefID associated with a given Operand (function)
pub fn get_operand_def_id<'tcx>(operand: &Operand<'tcx>) -> Option<DefId> {
    match operand {
        Operand::Constant(c) => {
            match &c.const_ {
                // Only `Const::Val` can hold a fully‑evaluated constant with its Ty.
                Const::Val(_val, ty) => {
                    if let TyKind::FnDef(def_id, _generic_args) = ty.kind() {
                        return Some(*def_id); // Preserve the full `DefId`
                    }
                }
                _ => {}
            }
            None
        }
        // `Copy` and `Move` operands can’t be `FnDef`s.
        Operand::Copy(_) | Operand::Move(_) => None,
    }
}

// Extract the bytes of a `&'static str` literal embedded in an `Operand`.
// Returns None when the operand is not a constant or its type is not
// `&str`, or when the bytes are not valid UTF‑8.
pub fn get_operand_const_string<'tcx>(operand: &Operand<'tcx>) -> Option<String> {
    // is it an `Operand::Constant`
    let (val, ty): (ConstValue<'tcx>, Ty<'tcx>) = match operand {
        Operand::Constant(c) => match c.const_ {
            // Already‑evaluated constant
            Const::Val(val, ty) => (val, ty),
            // `Const::Unevaluated` and `Const::Ty` need tcx to resolve , skipping for now
            _ => return None,
        },
        // Copy / Move refer to locals, not literals
        _ => return None,
    };

    // is it `&str` ?
    match ty.kind() {
        TyKind::Ref(_, inner, _) if matches!(inner.kind(), TyKind::Str) => {
            // Continue to byte extraction - don't return here
        }
        _ => return None,
    }

    // can we get the raw bytes?
    let bytes = match val {
        ConstValue::Slice { data, meta } => {
            let range = AllocRange {
                start: Size::from_bytes(0),
                size: Size::from_bytes(meta),
            };
            data.0.get_bytes_unchecked(range).to_vec()
        }
        // other `ConstValue`s (Scalar, ByRef, ZeroSized …) cannot encode a
        // string literal on nightly‑2024‑07‑22, so just give up!
        _ => return None,
    };

    match String::from_utf8(bytes) {
        Ok(s) => Some(s),
        Err(e) => None,
    }
}

// Get the `Local` associated with an Operand if of Move variant
pub fn get_operand_local<'tcx>(operand: &Operand<'tcx>) -> Option<usize> {
    match operand {
        // In optimized MIR example 1 is a copy case, instead of a move case
        Operand::Copy(place) => {
            let local = place.local;
            let projection = place.projection;
            Some(local.as_usize())
        }
        Operand::Move(place) => {
            let local = place.local;
            let projection = place.projection;
            Some(local.as_usize())
        }
        Operand::Constant(place) => {
            Some(0)
            // None
        }
    }
}

pub fn extract_string_from_const<'tcx>(
    data: &'tcx ConstAllocation<'tcx>, //pub struct ConstAllocation<'tcx>(pub Interned<'tcx, Allocation>);
    meta: u64,
) -> Option<String> {
    println!("\tData: {:?}", data);
    println!("\tMeta: {:?}", meta);

    //0: Interned<'tcx, Allocation>
    let range: AllocRange = AllocRange {
        start: rustc_abi::Size::from_bytes(0),
        size: rustc_abi::Size::from_bytes(meta),
    };
    let allocation = &data.0.get_bytes_unchecked(range); //this is alignment

    // let a: String = String::from_utf8(allocation.to_vec()).unwrap();
    // avoid unwrap, handle error
    let a: String = match String::from_utf8(allocation.to_vec()) {
        Ok(s) => s,
        Err(_) => {
            println!("extract_string_from_const: Failed to convert bytes to String");
            return None;
        }
    };

    return Some(a);
}

pub fn get_operand_span(operand: &Operand) -> Option<rustc_span::Span> {
    match operand {
        Operand::Copy(_place) => {
            println!("get_operand_span: Unsupported, This function currently caters only for constants. ");
            return None;
        }
        Operand::Move(place) => {
            println!("get_operand_span: Unsupported, This function currently caters only for constants. ");
            None
        }
        Operand::Constant(place) => {
            let const_span = place.span;
            return Some(const_span);
        }
    }
}
