pub fn example(val: f64) -> &'static str {
    if val >= 50.0 {
        "foo"
    } else {
        "bar"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_example() {
        assert_eq!(example(75.0), "foo");
        assert_eq!(example(90.0), "foo");
    }
}
