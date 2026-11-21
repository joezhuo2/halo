use eframe::egui::{Pos2, Rect, pos2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    pub fn row(self) -> bool {
        matches!(self, Dir::Left | Dir::Right)
    }

    fn after(self) -> bool {
        matches!(self, Dir::Right | Dir::Down)
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Node {
    Leaf(usize),
    Split(bool, Vec<(f32, Node)>),
}

pub struct Splitter {
    pub path: Vec<usize>,
    pub index: usize,
    pub row: bool,
    pub rect: Rect,
    pub extent: f32,
}

pub fn line(row: bool, kids: impl IntoIterator<Item = Node>) -> Node {
    let kids: Vec<Node> = kids.into_iter().collect();
    let share = 1.0 / kids.len() as f32;
    Node::Split(row, kids.into_iter().map(|k| (share, k)).collect())
}

fn flatten(row: bool, kids: Vec<(f32, Node)>) -> Vec<(f32, Node)> {
    kids.into_iter()
        .flat_map(|(share, kid)| match kid {
            Node::Split(r, inner) if r == row => {
                let total: f32 = inner.iter().map(|k| k.0).sum();
                inner.into_iter().map(|(s, k)| (share * s / total, k)).collect()
            }
            kid => vec![(share, kid)],
        })
        .collect()
}

impl Node {
    pub fn leaves(&self) -> Vec<usize> {
        match self {
            Node::Leaf(g) => vec![*g],
            Node::Split(_, kids) => kids.iter().flat_map(|k| k.1.leaves()).collect(),
        }
    }

    pub fn contains(&self, g: usize) -> bool {
        match self {
            Node::Leaf(l) => *l == g,
            Node::Split(_, kids) => kids.iter().any(|k| k.1.contains(g)),
        }
    }

    pub fn map(self, f: &impl Fn(usize) -> usize) -> Node {
        match self {
            Node::Leaf(g) => Node::Leaf(f(g)),
            Node::Split(row, kids) => Node::Split(row, kids.into_iter().map(|(s, k)| (s, k.map(f))).collect()),
        }
    }

    fn walk(&self, rect: Rect, path: &mut Vec<usize>, leaves: &mut Vec<(usize, Rect)>, splitters: &mut Vec<Splitter>) {
        let Node::Split(row, kids) = self else {
            if let Node::Leaf(g) = self {
                leaves.push((*g, rect));
            }
            return;
        };
        let extent = if *row { rect.width() } else { rect.height() };
        let total: f32 = kids.iter().map(|k| k.0).sum();
        let mut at = if *row { rect.left() } else { rect.top() };
        for (i, (share, kid)) in kids.iter().enumerate() {
            let end = if i + 1 == kids.len() { if *row { rect.right() } else { rect.bottom() } } else { at + extent * share / total };
            let part = match row {
                true => Rect::from_x_y_ranges(at..=end, rect.y_range()),
                false => Rect::from_x_y_ranges(rect.x_range(), at..=end),
            };
            path.push(i);
            kid.walk(part, path, leaves, splitters);
            path.pop();
            if i + 1 < kids.len() {
                let rect = match row {
                    true => Rect::from_x_y_ranges(end..=end, rect.y_range()),
                    false => Rect::from_x_y_ranges(rect.x_range(), end..=end),
                };
                splitters.push(Splitter { path: path.clone(), index: i, row: *row, rect, extent });
            }
            at = end;
        }
    }

    pub fn rects(&self, rect: Rect) -> Vec<(usize, Rect)> {
        let mut leaves = vec![];
        self.walk(rect, &mut vec![], &mut leaves, &mut vec![]);
        leaves
    }

    pub fn splitters(&self, rect: Rect) -> Vec<Splitter> {
        let mut splitters = vec![];
        self.walk(rect, &mut vec![], &mut vec![], &mut splitters);
        splitters
    }

    pub fn split(&mut self, g: usize, new: usize, dir: Dir) -> bool {
        match self {
            Node::Leaf(l) if *l == g => {
                let pair = if dir.after() { [g, new] } else { [new, g] };
                *self = line(dir.row(), pair.map(Node::Leaf));
                true
            }
            Node::Leaf(_) => false,
            Node::Split(row, kids) => {
                if *row == dir.row()
                    && let Some(i) = kids.iter().position(|k| k.1 == Node::Leaf(g))
                {
                    kids[i].0 /= 2.0;
                    kids.insert(i + dir.after() as usize, (kids[i].0, Node::Leaf(new)));
                    return true;
                }
                kids.iter_mut().any(|k| k.1.split(g, new, dir))
            }
        }
    }

    pub fn without(self, g: usize) -> Option<Node> {
        match self {
            Node::Leaf(l) if l == g => None,
            Node::Leaf(l) => Some(Node::Leaf(if l > g { l - 1 } else { l })),
            Node::Split(row, kids) => {
                let mut kept: Vec<(f32, Node)> = kids.into_iter().filter_map(|(s, k)| k.without(g).map(|k| (s, k))).collect();
                let total: f32 = kept.iter().map(|k| k.0).sum();
                kept.iter_mut().for_each(|k| k.0 /= total);
                match kept.len() {
                    0 => None,
                    1 => kept.pop().map(|k| k.1),
                    _ => Some(Node::Split(row, flatten(row, kept))),
                }
            }
        }
    }

    fn at_mut(&mut self, path: &[usize]) -> Option<&mut Node> {
        match (path.split_first(), self) {
            (None, node) => Some(node),
            (Some((i, rest)), Node::Split(_, kids)) => kids.get_mut(*i)?.1.at_mut(rest),
            _ => None,
        }
    }

    pub fn resize(&mut self, path: &[usize], index: usize, delta: f32, min: f32) {
        let Some(Node::Split(_, kids)) = self.at_mut(path) else { return };
        let (Some(a), Some(b)) = (kids.get(index).map(|k| k.0), kids.get(index + 1).map(|k| k.0)) else { return };
        if a + b < min * 2.0 {
            return;
        }
        let d = delta.max(min - a).min(b - min);
        kids[index].0 = a + d;
        kids[index + 1].0 = b - d;
    }

    pub fn even(&mut self, path: Option<&[usize]>) {
        let deep = path.is_none();
        let Some(Node::Split(_, kids)) = self.at_mut(path.unwrap_or_default()) else { return };
        let share = 1.0 / kids.len() as f32;
        for kid in kids {
            kid.0 = share;
            if deep {
                kid.1.even(None);
            }
        }
    }

    pub fn neighbor(&self, g: usize, dir: Dir) -> Option<usize> {
        let rects = self.rects(unit());
        let r = rects.iter().find(|(l, _)| *l == g)?.1;
        let overlap = |a: (f32, f32), b: (f32, f32)| a.1.min(b.1) - a.0.max(b.0);
        let eps = 1e-4;
        rects
            .iter()
            .filter(|(l, _)| *l != g)
            .filter_map(|(l, c)| {
                let (edge, span) = match dir {
                    Dir::Left => (c.right() - r.left(), overlap((c.top(), c.bottom()), (r.top(), r.bottom()))),
                    Dir::Right => (c.left() - r.right(), overlap((c.top(), c.bottom()), (r.top(), r.bottom()))),
                    Dir::Up => (c.bottom() - r.top(), overlap((c.left(), c.right()), (r.left(), r.right()))),
                    Dir::Down => (c.top() - r.bottom(), overlap((c.left(), c.right()), (r.left(), r.right()))),
                };
                (edge.abs() < eps && span > eps).then_some((*l, span))
            })
            .reduce(|a, b| if b.1 > a.1 + eps { b } else { a })
            .map(|(l, _)| l)
    }
}

pub fn zone(rect: Rect, pos: Pos2) -> Option<Dir> {
    let x = (pos.x - rect.left()) / rect.width();
    let y = (pos.y - rect.top()) / rect.height();
    [(x, Dir::Left), (1.0 - x, Dir::Right), (y, Dir::Up), (1.0 - y, Dir::Down)]
        .into_iter()
        .filter(|(d, _)| *d < 0.3)
        .reduce(|a, b| if b.0 < a.0 { b } else { a })
        .map(|(_, dir)| dir)
}

pub fn half(rect: Rect, dir: Option<Dir>) -> Rect {
    let c = rect.center();
    match dir {
        None => rect,
        Some(Dir::Left) => rect.with_max_x(c.x),
        Some(Dir::Right) => rect.with_min_x(c.x),
        Some(Dir::Up) => rect.with_max_y(c.y),
        Some(Dir::Down) => rect.with_min_y(c.y),
    }
}

pub fn unit() -> Rect {
    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use Node::Leaf;
    use eframe::egui::vec2;

    fn grid() -> Node {
        line(false, [line(true, [Leaf(0), Leaf(1)]), line(true, [Leaf(2), Leaf(3)])])
    }

    #[test]
    fn splits_in_a_direction_and_reuses_a_parent_of_the_same_kind() {
        let mut node = Leaf(0);
        assert!(node.split(0, 1, Dir::Right));
        assert_eq!(node, line(true, [Leaf(0), Leaf(1)]));
        assert!(node.split(0, 2, Dir::Left));
        assert_eq!(node, Node::Split(true, vec![(0.25, Leaf(2)), (0.25, Leaf(0)), (0.5, Leaf(1))]));
        assert!(node.split(1, 3, Dir::Down));
        assert_eq!(node.leaves(), vec![2, 0, 1, 3]);
        let Node::Split(_, kids) = &node else { panic!() };
        assert_eq!(kids[2].1, line(false, [Leaf(1), Leaf(3)]));
        assert!(!node.split(9, 4, Dir::Up));
    }

    #[test]
    fn removes_a_group_collapses_its_parent_and_renumbers() {
        let node = grid().without(1).unwrap();
        assert_eq!(node, line(false, [Leaf(0), line(true, [Leaf(1), Leaf(2)])]));
        let node = node.without(0).unwrap();
        assert_eq!(node, line(true, [Leaf(0), Leaf(1)]));
        assert_eq!(Leaf(0).without(0), None);
        let nested = line(true, [Leaf(0), line(false, [Leaf(1), line(true, [Leaf(2), Leaf(3)])])]);
        let flat = nested.without(1).unwrap();
        assert_eq!(flat, Node::Split(true, vec![(0.5, Leaf(0)), (0.25, Leaf(1)), (0.25, Leaf(2))]));
        let three = Node::Split(true, vec![(0.5, Leaf(0)), (0.25, Leaf(1)), (0.25, Leaf(2))]).without(0).unwrap();
        assert_eq!(three, Node::Split(true, vec![(0.5, Leaf(0)), (0.5, Leaf(1))]));
    }

    #[test]
    fn resizes_splitters_with_a_minimum_and_evens_them_out() {
        let mut node = grid();
        let splitters = node.splitters(Rect::from_min_size(Pos2::ZERO, vec2(200.0, 100.0)));
        assert_eq!(splitters.len(), 3);
        let top = splitters.iter().find(|s| s.path == [0]).unwrap();
        assert!(top.row && top.index == 0 && top.extent == 200.0 && top.rect.left() == 100.0 && top.rect.bottom() == 50.0);
        let middle = splitters.iter().find(|s| s.path.is_empty()).unwrap();
        assert!(!middle.row && middle.rect.top() == 50.0 && middle.rect.width() == 200.0);
        node.resize(&[0], 0, 0.3, 0.1);
        node.resize(&[], 0, -0.9, 0.1);
        let near = |a: Rect, b: Rect| (a.min - b.min).length() + (a.max - b.max).length() < 1e-3;
        let rects = node.rects(Rect::from_min_size(Pos2::ZERO, vec2(200.0, 100.0)));
        assert!(near(rects[0].1, Rect::from_min_max(pos2(0.0, 0.0), pos2(160.0, 10.0))), "{rects:?}");
        assert!(near(rects[3].1, Rect::from_min_max(pos2(100.0, 10.0), pos2(200.0, 100.0))));
        node.resize(&[0], 0, 0.9, 0.1);
        assert!((node.rects(unit())[1].1.width() - 0.1).abs() < 1e-4);
        node.even(Some(&[0]));
        assert_eq!(node.rects(unit())[1].1.width(), 0.5);
        assert!((node.rects(unit())[1].1.height() - 0.1).abs() < 1e-4);
        node.even(None);
        assert_eq!(node, grid());
    }

    #[test]
    fn finds_neighbours_by_direction_in_a_grid() {
        let node = grid();
        assert_eq!(node.neighbor(0, Dir::Right), Some(1));
        assert_eq!(node.neighbor(0, Dir::Down), Some(2));
        assert_eq!(node.neighbor(0, Dir::Left), None);
        assert_eq!(node.neighbor(0, Dir::Up), None);
        assert_eq!(node.neighbor(3, Dir::Left), Some(2));
        assert_eq!(node.neighbor(3, Dir::Up), Some(1));
        let right = line(true, [Leaf(0), line(false, [Leaf(1), Leaf(2)])]);
        assert_eq!(right.neighbor(2, Dir::Left), Some(0));
        assert_eq!(right.neighbor(0, Dir::Right), Some(1));
        assert_eq!(right.neighbor(5, Dir::Right), None);
    }

    #[test]
    fn serializes_and_maps_leaves() {
        let node = grid();
        let text = serde_json::to_string(&node).unwrap();
        assert_eq!(serde_json::from_str::<Node>(&text).unwrap(), node);
        assert_eq!(node.map(&|g| g + 10).leaves(), vec![10, 11, 12, 13]);
    }

    #[test]
    fn resolves_the_five_drop_zones() {
        let rect = Rect::from_min_size(pos2(100.0, 0.0), vec2(200.0, 100.0));
        assert_eq!(zone(rect, pos2(110.0, 50.0)), Some(Dir::Left));
        assert_eq!(zone(rect, pos2(290.0, 50.0)), Some(Dir::Right));
        assert_eq!(zone(rect, pos2(200.0, 5.0)), Some(Dir::Up));
        assert_eq!(zone(rect, pos2(200.0, 95.0)), Some(Dir::Down));
        assert_eq!(zone(rect, pos2(200.0, 50.0)), None);
        assert_eq!(zone(rect, pos2(105.0, 90.0)), Some(Dir::Left));
        assert_eq!(half(rect, Some(Dir::Down)), Rect::from_min_max(pos2(100.0, 50.0), pos2(300.0, 100.0)));
        assert_eq!(half(rect, None), rect);
    }
}
