use rustc_middle::mir::interpret::{AllocRange, ConstAllocation};
use rustc_middle::mir::{Const, ConstValue, Local, Place};
use rustc_middle::ty::ScalarInt;
use rustc_middle::ty::TyKind;

use rustc_middle::mir::Operand;

// Get the DefID associated with a given Operand (function)
pub fn get_operand_def_id<'tcx>(operand: &Operand<'tcx>) -> Option<usize> {
    match operand {
        Operand::Copy(_place) => {
            println!(
                "get_operand_def_id: This should never happen, contact Hassnain if this is printed"
            );
            None
        }
        Operand::Move(place) => None,
        Operand::Constant(place) => {
            let constant = place.const_;
            match constant {
                Const::Ty(_ty, _const) => {
                    println!("get_operand_def_id: This should never happen, contact Hassnain if this is printed");
                    None
                }
                Const::Unevaluated(_unevaluated_const, _ty) => {
                    println!("get_operand_def_id: This should never happen, contact Hassnain if this is printed");
                    None
                }
                Const::Val(const_value, ty) => {
                    if let TyKind::FnDef(def_id, idk) = ty.kind() {
                        return Some(def_id.index.as_usize());
                    }
                    None
                }
            }
        }
    }
}

// Get the constant string from an Operand::Constant.const_ of type Const::Val
pub fn get_operand_const_string<'tcx>(operand: &Operand<'tcx>) -> Option<String> {
    match operand {
        Operand::Copy(_place) => {
            println!(
                "get_operand_const_string: This should never happen, contact Hassnain if this is printed"
            );
            None
        }
        Operand::Move(place) => {
            println!(
                "get_operand_const_string: This should never happen, contact Hassnain if this is printed"
            );
            None
        }
        Operand::Constant(place) => {
            let constant = place.const_;
            match constant {
                Const::Ty(_ty, _const) => {
                    println!("get_operand_const_string: This should never happen, contact Hassnain if this is printed");
                    None
                }
                Const::Unevaluated(_unevaluated_const, _ty) => {
                    println!("get_operand_const_string: This should never happen, contact Hassnain if this is printed");
                    None
                }
                Const::Val(const_value, ty) => {
                    match const_value {
                        ConstValue::Slice { data, meta } => {
                            if let Some(str_data) = extract_string_from_const(&data, meta) {
                                return Some(str_data);
                            }
                        }
                        _ => {
                            println!("get_operand_const_string: This should never happen, contact Hassnain if this is printed");
                        }
                    }
                    None
                }
            }
        }
    }
}

// Get the `Local` associated with an Operand if of Move variant
pub fn get_operand_local<'tcx>(operand: &Operand<'tcx>) -> Option<usize> {
    match operand {
        Operand::Copy(_place) => {
            println!(
                "get_operand_local: This should never happen, contact Hassnain if this is printed"
            );
            None
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

    let a: String = String::from_utf8(allocation.to_vec()).unwrap();

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

