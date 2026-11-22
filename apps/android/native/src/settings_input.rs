use std::str::FromStr;

pub(crate) fn parse_number<Number: FromStr>(input: &str) -> Result<Number, Number::Err> {
    input.trim().parse()
}

#[cfg(test)]
mod tests {
    use super::parse_number;

    #[test]
    fn ports_and_speed_test_intervals_accept_surrounding_whitespace() {
        assert_eq!(parse_number::<u16>(" \t7890 "), Ok(7890));
        assert_eq!(parse_number::<u32>(" 15\n"), Ok(15));
        assert_eq!(parse_number::<u32>(" 0 "), Ok(0));
        assert!(parse_number::<u16>(" 65536 ").is_err());
        assert!(parse_number::<u16>(" 78 90 ").is_err());
        assert!(parse_number::<u32>(" \t ").is_err());
        assert!(parse_number::<u32>(" -1 ").is_err());
    }
}
