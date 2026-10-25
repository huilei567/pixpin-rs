//! 版面还原：把一堆「行」拼回段落，并决定哪里该换行、哪里该补空格。
//!
//! PixPin 这一步用的是图神经网络（EdgeGNN），输入是行的几何特征。
//! 这里用规则版替代：行聚类成段 → 段内合并 → 中英文分别处理。
//! 规则版在截图这种版式简单的场景下能拿到接近的效果，而且没有模型依赖。

use crate::geom::Rect;

#[derive(Clone, Debug)]
pub struct Line {
    pub text: String,
    pub score: f32,
    pub rect: Rect,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub lines: Vec<Line>,
    pub rect: Rect,
}

impl Block {
    /// 段落文本。同一段内的多个视觉行会合并成一行（和 PDF 复制出来的行为一致）。
    pub fn text(&self) -> String {
        let mut out = String::new();
        for (i, line) in self.lines.iter().enumerate() {
            if i == 0 {
                out.push_str(&line.text);
            } else {
                out.push_str(&joiner(&out, &line.text));
                out.push_str(&line.text);
            }
        }
        clean_cjk_spaces(&out)
    }
}

struct Row {
    lines: Vec<Line>,
    rect: Rect,
}

/// 正在累积的段落：除了行本身，还要记住已观察到的行距，
/// 用来判断下一行是不是「行距突变」的另起一段。
struct Acc {
    rows: Vec<Row>,
    rect: Rect,
    gaps: Vec<i32>,
}

/// 行 → 段落。输入假定已按阅读顺序排好。
pub fn group(lines: Vec<Line>) -> Vec<Block> {
    if lines.is_empty() {
        return Vec::new();
    }
    let rows = build_rows(lines);
    let mut blocks: Vec<Acc> = Vec::new();

    for row in rows {
        let fits = blocks
            .last()
            .map(|acc| same_paragraph(acc, &row))
            .unwrap_or(false);
        if fits {
            let acc = blocks.last_mut().unwrap();
            let prev = acc.rows.last().unwrap().rect;
            acc.gaps.push(row.rect.y - prev.bottom());
            acc.rect = acc.rect.union(&row.rect);
            acc.rows.push(row);
        } else {
            blocks.push(Acc {
                rect: row.rect,
                gaps: Vec::new(),
                rows: vec![row],
            });
        }
    }

    blocks
        .into_iter()
        .map(|acc| {
            let mut all = Vec::new();
            for row in acc.rows {
                all.extend(row.lines);
            }
            Block {
                lines: all,
                rect: acc.rect,
            }
        })
        .collect()
}

/// 按垂直重叠把行聚成「视觉行」。
fn build_rows(mut lines: Vec<Line>) -> Vec<Row> {
    lines.sort_by(|a, b| {
        a.rect
            .center_y()
            .partial_cmp(&b.rect.center_y())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut rows: Vec<Row> = Vec::new();
    for line in lines {
        let rect = line.rect;
        let hit = rows.last().and_then(|row| {
            row.lines
                .iter()
                .any(|l| l.rect.vertical_overlap(&rect) > 0.5)
                .then(|| rows.len() - 1)
        });
        match hit {
            Some(i) => {
                rows[i].rect = rows[i].rect.union(&rect);
                rows[i].lines.push(line);
            }
            None => rows.push(Row {
                rect,
                lines: vec![line],
            }),
        }
    }

    for row in rows.iter_mut() {
        row.lines.sort_by_key(|l| l.rect.x);
        // 同一视觉行里的多个片段，间隙大才补空格
        let h = row.rect.h.max(1) as f32;
        let mut joined = String::new();
        for (i, l) in row.lines.iter().enumerate() {
            if i > 0 {
                let prev = &row.lines[i - 1];
                let gap = (l.rect.x - prev.rect.right()) as f32;
                if gap > 0.3 * h {
                    joined.push(' ');
                }
            }
            joined.push_str(&l.text);
        }
        // 复用第一行的位置放合并结果，后面拼段落时只关心文本
        let score = row.lines.iter().map(|l| l.score).sum::<f32>() / row.lines.len() as f32;
        let rect = row.rect;
        row.lines = vec![Line {
            text: joined,
            score,
            rect,
        }];
    }
    rows
}

/// 判断下一行是否还属于当前段落。
///
/// 只看「行距」不够——同一页里标题、正文、代码块的行距各不相同，
/// 但同一段内部的行距是稳定的。所以除了绝对阈值，还看行距是否突变。
fn same_paragraph(acc: &Acc, next: &Row) -> bool {
    let prev = acc.rows.last().unwrap().rect;
    let h = prev.h.max(next.rect.h).max(1) as f32;
    let gap = (next.rect.y - prev.bottom()) as f32;

    // 行间距过大或明显重叠，都不是同一段的正常形态
    if gap > 1.6 * h || gap < -0.6 * h {
        return false;
    }
    // 左边界对不齐：列表、表格、缩进段落
    if (next.rect.x - acc.rect.x).abs() as f32 > 0.8 * h {
        return false;
    }
    // 字号突变：标题 → 正文
    let size_delta = (prev.h - next.rect.h).abs() as f32 / h;
    if size_delta > 0.35 {
        return false;
    }
    // 行距突变：本段已建立的行距节奏被打断
    if !acc.gaps.is_empty() {
        let mut g = acc.gaps.clone();
        g.sort_unstable();
        let med = g[(g.len() - 1) / 2] as f32;
        if med > 0.0 && gap > med * 1.45 + 2.0 {
            return false;
        }
    }
    true
}

/// 两段文本之间的连接符。
fn joiner(prev: &str, next: &str) -> &'static str {
    let last = prev.chars().last();
    let first = next.chars().next();
    match (last, first) {
        (Some('-'), _) => "",
        // 两侧都是汉字才直接接上；只要有一侧是 ASCII 就补空格，
        // 否则 "}" 和下一行开头的 "检" 会粘成 "}检"。
        (Some(a), Some(b)) if is_cjk(a) && is_cjk(b) => "",
        (Some(_), Some(_)) => " ",
        _ => "",
    }
}

fn is_cjk(c: char) -> bool {
    let u = c as u32;
    (0x2E80..=0x9FFF).contains(&u)      // 部首 + 中日韩统一表意
        || (0xF900..=0xFAFF).contains(&u) // 兼容表意
        || (0xFF00..=0xFFEF).contains(&u) // 全角标点
        || (0x3000..=0x303F).contains(&u) // 中日韩标点
        || (0xAC00..=0xD7AF).contains(&u) // 谚文
        || (0x3040..=0x30FF).contains(&u) // 假名
}

/// 去掉两个汉字之间被 OCR 塞进去的空格（英文单词之间的空格要保留）。
pub fn clean_cjk_spaces(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == ' ' {
            let prev = chars[..i].iter().rev().find(|x| **x != ' ');
            let next = chars[i + 1..].iter().find(|x| **x != ' ');
            if let (Some(p), Some(n)) = (prev, next) {
                if is_cjk(*p) && is_cjk(*n) {
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}
