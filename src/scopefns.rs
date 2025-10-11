pub trait Also: Sized {
    fn also(self, f: impl FnOnce(&Self)) -> Self;
    fn apply(self, f: impl FnOnce(Self) -> Self) -> Self;
}

impl<T> Also for T {
    fn also(self, f: impl FnOnce(&Self)) -> Self {
        f(&self);
        self
    }

    fn apply(self, f: impl FnOnce(Self) -> Self) -> Self {
        f(self)
    }
}
