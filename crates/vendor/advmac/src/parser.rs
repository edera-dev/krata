use crate::ParseError;

// This whole thing is written this way to be const.
// If you want normal hex handling, just use hex crate
pub struct MacParser<const N: usize, const N2: usize>;

impl<const N: usize, const N2: usize> MacParser<N, N2> {
    const CANONICAL_COLON_SIZE: usize = 3 * N - 1;
    const DOT_NOTATION_SIZE: usize = (2 * N) + (N / 2 - 1);
    const HEXADECIMAL_SIZE: usize = 2 * N;
    const HEXADECIMAL0X_SIZE: usize = 2 * N + 2;

    #[inline]
    const fn nibble(v: u8) -> Result<u8, ParseError> {
        match v {
            b'A'..=b'F' => Ok(10 + (v - b'A')),
            b'a'..=b'f' => Ok(10 + (v - b'a')),
            b'0'..=b'9' => Ok(v - b'0'),
            _ => Err(ParseError::InvalidMac),
        }
    }

    #[inline]
    const fn byte(b1: u8, b2: u8) -> Result<u8, ParseError> {
        // ? is not available in const
        match (Self::nibble(b1), Self::nibble(b2)) {
            (Ok(v1), Ok(v2)) => Ok((v1 << 4) + v2),
            (Err(e), _) | (_, Err(e)) => Err(e),
        }
    }

    const fn from_hex(s: &[u8]) -> Result<[u8; N], ParseError> {
        if s.len() != Self::HEXADECIMAL_SIZE {
            return Err(ParseError::InvalidLength { length: s.len() });
        }

        let mut result = [0u8; N];

        // for-loops and iterators are unavailable in const
        let mut i = 0;
        while i < N {
            result[i] = match Self::byte(s[2 * i], s[2 * i + 1]) {
                Ok(v) => v,
                Err(e) => return Err(e),
            };
            i += 1;
        }

        Ok(result)
    }

    const fn check_separator(s: &[u8], sep: u8, group_len: usize) -> bool {
        let mut i = group_len;
        while i < s.len() {
            if s[i] != sep {
                return false;
            }
            i += group_len + 1;
        }
        true
    }

    const fn parse_separated(s: &[u8], sep: u8, group_len: usize) -> Result<[u8; N], ParseError> {
        let expected_len = (2 * N) + ((2 * N) / group_len) - 1;
        if s.len() != expected_len {
            return Err(ParseError::InvalidLength { length: s.len() });
        }

        if !Self::check_separator(s, sep, group_len) {
            return Err(ParseError::InvalidMac);
        }

        let mut hex_buf = [0u8; N2];

        let (mut in_i, mut out_i) = (0, 0);
        while in_i < s.len() {
            if (in_i + 1) % (group_len + 1) != 0 {
                hex_buf[out_i] = s[in_i];
                out_i += 1;
            }
            in_i += 1;
        }

        Self::from_hex(&hex_buf)
    }

    pub const fn parse(s: &str) -> Result<[u8; N], ParseError> {
        let s = s.as_bytes();

        if s.len() == Self::HEXADECIMAL_SIZE {
            Self::from_hex(s)
        } else if (s.len() == Self::HEXADECIMAL0X_SIZE) && (s[0] == b'0') && (s[1] == b'x') {
            // unsafe is the only way I know to make it const
            Self::from_hex(unsafe {
                core::slice::from_raw_parts(s.as_ptr().offset(2), s.len() - 2)
            })
        } else if s.len() == Self::CANONICAL_COLON_SIZE {
            let sep = s[2];
            match sep {
                b'-' | b':' => Self::parse_separated(s, sep, 2),
                _ => Err(ParseError::InvalidMac),
            }
        } else if s.len() == Self::DOT_NOTATION_SIZE {
            let sep = s[4];
            match sep {
                b'.' => Self::parse_separated(s, sep, 4),
                _ => Err(ParseError::InvalidMac),
            }
        } else {
            Err(ParseError::InvalidLength { length: s.len() })
        }
    }
}
