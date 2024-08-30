#[derive(Debug)]
struct Food {
    protein: usize,
    name: String,
}

fn foo() {
    let a: i32 = 70;
    if a == 71 {
        let b = Food {protein: 8, name: "egg".to_string()};
        println!("{:#?}", b);
    } 
}