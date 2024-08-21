fn main(x: i32) -> i32 {
    let y: i32;
    match x {
        0 => y = 1,
        1 => y = 3,
        2 => y = 9,
        _ => y = 11,
    }
    y
}