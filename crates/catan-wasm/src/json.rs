//! 最小の JSON 書き出し。読み込みは要らない（UI は合法手を番号で返すので）。

pub struct Json {
    buf: String,
    /// 直前にカンマが要るか
    need_comma: bool,
}

impl Json {
    pub fn new() -> Self {
        Json {
            buf: String::with_capacity(64 * 1024),
            need_comma: false,
        }
    }

    fn sep(&mut self) {
        if self.need_comma {
            self.buf.push(',');
        }
        self.need_comma = true;
    }

    pub fn obj_start(&mut self) {
        self.sep();
        self.buf.push('{');
        self.need_comma = false;
    }
    pub fn obj_end(&mut self) {
        self.buf.push('}');
        self.need_comma = true;
    }
    pub fn arr_start(&mut self) {
        self.buf.push('[');
        self.need_comma = false;
    }
    pub fn arr_end(&mut self) {
        self.buf.push(']');
        self.need_comma = true;
    }

    /// オブジェクトのキー。この直後の値には区切りを打たない
    pub fn key(&mut self, k: &str) {
        self.sep();
        self.buf.push('"');
        self.buf.push_str(k);
        self.buf.push_str("\":");
        self.need_comma = false;
    }

    pub fn str(&mut self, s: &str) {
        self.sep();
        self.buf.push('"');
        for c in s.chars() {
            match c {
                '"' => self.buf.push_str("\\\""),
                '\\' => self.buf.push_str("\\\\"),
                '\n' => self.buf.push_str("\\n"),
                '\r' => self.buf.push_str("\\r"),
                '\t' => self.buf.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    self.buf.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => self.buf.push(c),
            }
        }
        self.buf.push('"');
    }

    pub fn num(&mut self, v: f32) {
        self.sep();
        if v.fract() == 0.0 && v.abs() < 1e9 {
            self.buf.push_str(&format!("{}", v as i64));
        } else {
            self.buf.push_str(&format!("{v:.2}"));
        }
    }

    pub fn bool(&mut self, v: bool) {
        self.sep();
        self.buf.push_str(if v { "true" } else { "false" });
    }

    /// 既に JSON になっているものをそのまま入れる
    pub fn raw(&mut self, s: &str) {
        self.sep();
        self.buf.push_str(s);
    }

    pub fn finish(self) -> String {
        self.buf
    }
}

impl Default for Json {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 入れ子と区切りが正しい() {
        let mut j = Json::new();
        j.obj_start();
        j.key("a");
        j.num(1.0);
        j.key("b");
        j.arr_start();
        j.num(1.0);
        j.num(2.0);
        j.arr_end();
        j.key("c");
        j.obj_start();
        j.key("d");
        j.str("x");
        j.obj_end();
        j.obj_end();
        assert_eq!(j.finish(), r#"{"a":1,"b":[1,2],"c":{"d":"x"}}"#);
    }

    #[test]
    fn 日本語と記号を壊さない() {
        let mut j = Json::new();
        j.obj_start();
        j.key("s");
        j.str("木材\"1\"・改行\n有り");
        j.obj_end();
        let out = j.finish();
        assert!(out.contains("木材"));
        assert!(out.contains("\\\""));
        assert!(out.contains("\\n"));
    }

    #[test]
    fn 配列の中のオブジェクトが並ぶ() {
        let mut j = Json::new();
        j.arr_start();
        for i in 0..3 {
            j.obj_start();
            j.key("i");
            j.num(i as f32);
            j.obj_end();
        }
        j.arr_end();
        assert_eq!(j.finish(), r#"[{"i":0},{"i":1},{"i":2}]"#);
    }
}
