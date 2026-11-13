//! Chunked node storage (design §3.3). Growing never moves existing nodes, so there is no
//! transient copy of the whole tree as with a doubling `Vec`.

use super::Node;
use crate::scan::NodeId;

const CHUNK_BITS: u32 = 16;
const CHUNK: usize = 1 << CHUNK_BITS;
const MASK: u32 = (1 << CHUNK_BITS) - 1;

#[derive(Default)]
pub(crate) struct Arena {
    chunks: Vec<Vec<Node>>,
    len: u32,
}

impl Arena {
    pub fn len(&self) -> u32 {
        self.len
    }

    /// Grows to at least `len` nodes; new slots are vacant.
    pub fn ensure_len(&mut self, len: u32) {
        while self.len < len {
            self.push(Node::vacant());
        }
    }

    /// Appends a node and returns its id.
    pub fn push(&mut self, node: Node) -> NodeId {
        let id = NodeId(self.len);
        if (self.len & MASK) == 0 {
            self.chunks.push(Vec::with_capacity(CHUNK));
        }
        self.chunks.last_mut().expect("chunk exists").push(node);
        self.len += 1;
        id
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        (id.0 < self.len)
            .then(|| &self.chunks[(id.0 >> CHUNK_BITS) as usize][(id.0 & MASK) as usize])
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        (id.0 < self.len)
            .then(|| &mut self.chunks[(id.0 >> CHUNK_BITS) as usize][(id.0 & MASK) as usize])
    }

    /// All nodes in id order, consuming the arena.
    pub fn into_nodes(self) -> impl Iterator<Item = Node> {
        self.chunks.into_iter().flatten()
    }
}

impl std::ops::Index<NodeId> for Arena {
    type Output = Node;

    fn index(&self, id: NodeId) -> &Node {
        self.get(id)
            .unwrap_or_else(|| panic!("node {} out of range ({} nodes)", id.0, self.len))
    }
}

impl std::ops::IndexMut<NodeId> for Arena {
    fn index_mut(&mut self, id: NodeId) -> &mut Node {
        let len = self.len;
        self.get_mut(id)
            .unwrap_or_else(|| panic!("node {} out of range ({len} nodes)", id.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_across_chunks_without_moving_nodes() {
        let mut arena = Arena::default();
        arena.ensure_len(3);
        let first: *const Node = &arena[NodeId(0)];
        arena.ensure_len(CHUNK as u32 * 2 + 5);
        assert_eq!(arena.len(), CHUNK as u32 * 2 + 5);
        assert!(std::ptr::eq(first, &arena[NodeId(0)]));
        assert!(arena.get(NodeId(CHUNK as u32 * 2 + 4)).is_some());
        assert!(arena.get(NodeId(CHUNK as u32 * 2 + 5)).is_none());
        assert_eq!(arena.into_nodes().count(), CHUNK * 2 + 5);
    }
}
