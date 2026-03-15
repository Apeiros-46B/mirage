pub fn extent_of(win: &Window) -> Extent2D {
    let inner = win.inner_size();
    Extent2D { width: inner.width, height: inner.height }
}
