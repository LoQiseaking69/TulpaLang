
// Standard library for Tulpa language

pub fn print(text: &str) {
    println!("{}", text);
}

pub fn input(prompt: &str) -> String {
    use std::io::Write;
    print!("{}", prompt);
    std::io::stdout().flush().unwrap();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
    input.trim().to_string()
}

pub fn randint(low: i32, high: i32) -> i32 {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    rng.gen_range(low..=high)
}

pub fn read_file(filename: &str) -> std::io::Result<String> {
    use std::fs::File;
    use std::io::{self, Read};
    let mut file = File::open(filename)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    Ok(contents)
}

pub fn write_file(filename: &str, contents: &str) -> std::io::Result<()> {
    use std::fs::File;
    use std::io::Write;
    let mut file = File::create(filename)?;
    file.write_all(contents.as_bytes())?;
    Ok(())
}

pub fn create_dict<K: std::cmp::Eq + std::hash::Hash, V>() -> std::collections::HashMap<K, V> {
    std::collections::HashMap::new()
}

pub fn add_to_dict<K: std::cmp::Eq + std::hash::Hash, V>(dict: &mut std::collections::HashMap<K, V>, key: K, value: V) {
    dict.insert(key, value);
}

pub fn get_from_dict<K: std::cmp::Eq + std::hash::Hash, V>(dict: &std::collections::HashMap<K, V>, key: &K) -> Option<&V> {
    dict.get(key)
}

pub fn sleep(seconds: u64) {
    std::thread::sleep(std::time::Duration::from_secs(seconds));
}
