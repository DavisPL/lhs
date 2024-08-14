fn main(x: i32) -> i32 {
    let y: i32;
    if x == 0 {
        y = 1;
    } else if x == 1 {
        y = 3;
    } else if x == 2 {
        y = 9;
    } else {
        y = 11;
    }
    y
}