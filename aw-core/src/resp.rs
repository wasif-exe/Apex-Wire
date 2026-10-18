use std::io::Write as _;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command<'a> {
    Ping,
    Get(&'a [u8]),
    Set(&'a [u8], &'a [u8]),
    Del(&'a [u8]),
    Exists(&'a [u8]),
    Unknown,
}
pub struct RespParser;
impl RespParser {
    pub fn parse_commands<'a>(mut input: &'a [u8]) -> (Vec<Command<'a>>, usize) {
        let mut commands = Vec::new();
        let initial_len = input.len();
        while !input.is_empty() {
            if input.starts_with(b"*") {
                match Self::parse_array(input) {
                    Some((cmd, remaining)) => {
                        commands.push(cmd);
                        input = remaining;
                    }
                    None => break,
                }
            } else {
                match Self::parse_inline(input) {
                    Some((cmd, remaining)) => {
                        commands.push(cmd);
                        input = remaining;
                    }
                    None => break,
                }
            }
        }
        let consumed = initial_len - input.len();
        (commands, consumed)
    }
    fn parse_inline<'a>(input: &'a [u8]) -> Option<(Command<'a>, &'a [u8])> {
        let line_end = input.windows(2).position(|w| w == b"\r\n")?;
        let line = &input[..line_end];
        let remaining = &input[line_end + 2..];
        let mut parts = line.split(|&b| b == b' ').filter(|p| !p.is_empty());
        let cmd_name = parts.next()?;
        let cmd = if cmd_name.eq_ignore_ascii_case(b"PING") {
            Command::Ping
        } else if cmd_name.eq_ignore_ascii_case(b"GET") {
            let key = parts.next()?;
            Command::Get(key)
        } else if cmd_name.eq_ignore_ascii_case(b"SET") {
            let key = parts.next()?;
            let val = parts.next()?;
            Command::Set(key, val)
        } else if cmd_name.eq_ignore_ascii_case(b"DEL") {
            let key = parts.next()?;
            Command::Del(key)
        } else if cmd_name.eq_ignore_ascii_case(b"EXISTS") {
            let key = parts.next()?;
            Command::Exists(key)
        } else {
            Command::Unknown
        };
        Some((cmd, remaining))
    }
    fn parse_array<'a>(input: &'a [u8]) -> Option<(Command<'a>, &'a [u8])> {
        if !input.starts_with(b"*") { return None; }
        let crlf = input.windows(2).position(|w| w == b"\r\n")?;
        let num_elements_str = std::str::from_utf8(&input[1..crlf]).ok()?;
        let num_elements: usize = num_elements_str.parse().ok()?;
        let mut curr = &input[crlf + 2..];
        let mut elements: Vec<&'a [u8]> = Vec::with_capacity(num_elements);
        for _ in 0..num_elements {
            if !curr.starts_with(b"$") { return None; }
            let len_crlf = curr.windows(2).position(|w| w == b"\r\n")?;
            let len_str = std::str::from_utf8(&curr[1..len_crlf]).ok()?;
            let str_len: usize = len_str.parse().ok()?;
            let str_start = len_crlf + 2;
            let str_end = str_start + str_len;
            if curr.len() < str_end + 2 || &curr[str_end..str_end + 2] != b"\r\n" {
                return None;
            }
            elements.push(&curr[str_start..str_end]);
            curr = &curr[str_end + 2..];
        }
        if elements.is_empty() { return None; }
        let cmd_name = elements[0];
        let cmd = if cmd_name.eq_ignore_ascii_case(b"PING") {
            Command::Ping
        } else if cmd_name.eq_ignore_ascii_case(b"GET") && elements.len() >= 2 {
            Command::Get(elements[1])
        } else if cmd_name.eq_ignore_ascii_case(b"SET") && elements.len() >= 3 {
            Command::Set(elements[1], elements[2])
        } else if cmd_name.eq_ignore_ascii_case(b"DEL") && elements.len() >= 2 {
            Command::Del(elements[1])
        } else if cmd_name.eq_ignore_ascii_case(b"EXISTS") && elements.len() >= 2 {
            Command::Exists(elements[1])
        } else {
            Command::Unknown
        };
        Some((cmd, curr))
    }
    #[inline(always)]
    pub fn encode_pong(out: &mut Vec<u8>) {
        out.extend_from_slice(b"+PONG\r\n");
    }
    #[inline(always)]
    pub fn encode_ok(out: &mut Vec<u8>) {
        out.extend_from_slice(b"+OK\r\n");
    }
    #[inline(always)]
    pub fn encode_bulk_string(out: &mut Vec<u8>, val: Option<&[u8]>) {
        match val {
            Some(bytes) => {
                let _ = write!(out, "${}\r\n", bytes.len());
                out.extend_from_slice(bytes);
                out.extend_from_slice(b"\r\n");
            }
            None => {
                out.extend_from_slice(b"$-1\r\n");
            }
        }
    }
    #[inline(always)]
    pub fn encode_integer(out: &mut Vec<u8>, val: i64) {
        let _ = write!(out, ":{}\r\n", val);
    }
    #[inline(always)]
    pub fn encode_error(out: &mut Vec<u8>, msg: &str) {
        let _ = write!(out, "-ERR {}\r\n", msg);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_inline_ping() {
        let (cmds, consumed) = RespParser::parse_commands(b"PING\r\n");
        assert_eq!(cmds, vec![Command::Ping]);
        assert_eq!(consumed, 6);
    }
    #[test]
    fn parse_resp_set_get_pipeline() {
        let pipeline = b"*3\r\n$3\r\nSET\r\n$3\r\nfoo\r\n$3\r\nbar\r\n*2\r\n$3\r\nGET\r\n$3\r\nfoo\r\n";
        let (cmds, consumed) = RespParser::parse_commands(pipeline);
        assert_eq!(cmds, vec![
            Command::Set(b"foo", b"bar"),
            Command::Get(b"foo"),
        ]);
        assert_eq!(consumed, pipeline.len());
    }
    #[test]
    fn encode_responses() {
        let mut out = Vec::new();
        RespParser::encode_pong(&mut out);
        assert_eq!(out, b"+PONG\r\n");
        out.clear();
        RespParser::encode_bulk_string(&mut out, Some(b"hello"));
        assert_eq!(out, b"$5\r\nhello\r\n");
        out.clear();
        RespParser::encode_bulk_string(&mut out, None);
        assert_eq!(out, b"$-1\r\n");
    }
}
