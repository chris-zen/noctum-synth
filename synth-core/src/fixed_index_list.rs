#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Link {
    prev: Option<usize>,
    next: Option<usize>,
    listed: bool,
}

pub struct FixedIndexList<const N: usize> {
    links: [Link; N],
    head: Option<usize>,
    tail: Option<usize>,
    len: usize,
}

impl<const N: usize> Default for FixedIndexList<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> FixedIndexList<N> {
    pub const fn new() -> Self {
        Self {
            links: [Link {
                prev: None,
                next: None,
                listed: false,
            }; N],
            head: None,
            tail: None,
            len: 0,
        }
    }

    pub fn clear(&mut self) {
        for link in &mut self.links {
            *link = Link::default();
        }
        self.head = None;
        self.tail = None;
        self.len = 0;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn contains(&self, index: usize) -> bool {
        self.links.get(index).is_some_and(|link| link.listed)
    }

    pub fn front(&self) -> Option<usize> {
        self.head
    }

    pub fn back(&self) -> Option<usize> {
        self.tail
    }

    pub fn push_back(&mut self, index: usize) -> bool {
        if index >= N || self.contains(index) {
            return false;
        }

        self.links[index] = Link {
            prev: self.tail,
            next: None,
            listed: true,
        };

        match self.tail {
            Some(tail) => self.links[tail].next = Some(index),
            None => self.head = Some(index),
        }

        self.tail = Some(index);
        self.len += 1;
        true
    }

    pub fn push_front(&mut self, index: usize) -> bool {
        if index >= N || self.contains(index) {
            return false;
        }

        self.links[index] = Link {
            prev: None,
            next: self.head,
            listed: true,
        };

        match self.head {
            Some(head) => self.links[head].prev = Some(index),
            None => self.tail = Some(index),
        }

        self.head = Some(index);
        self.len += 1;
        true
    }

    pub fn remove(&mut self, index: usize) -> bool {
        if index >= N || !self.links[index].listed {
            return false;
        }

        let link = self.links[index];
        match link.prev {
            Some(prev) => self.links[prev].next = link.next,
            None => self.head = link.next,
        }
        match link.next {
            Some(next) => self.links[next].prev = link.prev,
            None => self.tail = link.prev,
        }

        self.links[index] = Link::default();
        self.len -= 1;
        true
    }

    pub fn move_to_back(&mut self, index: usize) -> bool {
        if !self.contains(index) {
            return false;
        }
        if self.tail == Some(index) {
            return true;
        }

        self.remove(index);
        self.push_back(index)
    }

    pub fn pop_front(&mut self) -> Option<usize> {
        let index = self.head?;
        self.remove(index);
        Some(index)
    }

    pub fn pop_back(&mut self) -> Option<usize> {
        let index = self.tail?;
        self.remove(index);
        Some(index)
    }

    pub fn iter(&self) -> FixedIndexListIter<'_, N> {
        FixedIndexListIter {
            list: self,
            next: self.head,
        }
    }
}

pub struct FixedIndexListIter<'a, const N: usize> {
    list: &'a FixedIndexList<N>,
    next: Option<usize>,
}

impl<const N: usize> Iterator for FixedIndexListIter<'_, N> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        let index = self.next?;
        self.next = self.list.links[index].next;
        Some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::FixedIndexList;
    use std::vec;
    use std::vec::Vec;

    #[test]
    fn preserves_insertion_order() {
        let mut list = FixedIndexList::<4>::new();
        assert!(list.push_back(2));
        assert!(list.push_back(0));
        assert!(list.push_back(3));

        assert_eq!(list.iter().collect::<Vec<_>>(), vec![2, 0, 3]);
        assert_eq!(list.front(), Some(2));
        assert_eq!(list.back(), Some(3));
    }

    #[test]
    fn removes_from_middle() {
        let mut list = FixedIndexList::<4>::new();
        list.push_back(0);
        list.push_back(1);
        list.push_back(2);

        assert!(list.remove(1));
        assert_eq!(list.iter().collect::<Vec<_>>(), vec![0, 2]);
        assert!(!list.contains(1));
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn moves_existing_item_to_back() {
        let mut list = FixedIndexList::<4>::new();
        list.push_back(0);
        list.push_back(1);
        list.push_back(2);

        assert!(list.move_to_back(0));
        assert_eq!(list.iter().collect::<Vec<_>>(), vec![1, 2, 0]);
    }
}
