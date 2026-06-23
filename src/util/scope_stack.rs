use std::collections::HashMap;

#[derive(Debug, Clone)]
pub(crate) struct ScopeStack<T> {
    scopes: Vec<HashMap<String, T>>,
}

impl<T> ScopeStack<T> {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    pub fn enter(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn exit(&mut self) {
        self.scopes.pop();
    }

    pub fn get(&self, name: &str) -> Option<&T> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }

    pub fn define(&mut self, name: String, val: T) {
        self.scopes
            .last_mut()
            .expect("ScopeStack::define called on empty stack")
            .insert(name, val);
    }
}
