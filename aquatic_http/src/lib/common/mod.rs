pub fn num_digits_in_usize(mut number: usize) -> usize {
    let mut num_digits = 1usize;

    while number >= 10 {
        num_digits += 1;

        number /= 10;
    }

    num_digits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_num_digits_in_usize() {
        let f = num_digits_in_usize;

        assert_eq!(f(0), 1);
        assert_eq!(f(1), 1);
        assert_eq!(f(9), 1);
        assert_eq!(f(10), 2);
        assert_eq!(f(11), 2);
        assert_eq!(f(99), 2);
        assert_eq!(f(100), 3);
        assert_eq!(f(101), 3);
        assert_eq!(f(1000), 4);
    }
}
