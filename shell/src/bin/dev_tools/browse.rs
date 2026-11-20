use enzui_shell::i18n::tr;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]

pub(crate) enum Section {
    Ports,
    Lookup,
    Groups,
    Containers,
    ContainerPorts,
    ContainerMounts,
}
impl Section {
    pub(crate) fn index(self) -> usize {
        match self {
            Self::Ports => 0,
            Self::Lookup => 1,
            Self::Groups => 2,
            Self::Containers => 3,
            Self::ContainerPorts => 4,
            Self::ContainerMounts => 5,
        }
    }
    pub(crate) fn size(self) -> usize {
        match self {
            Self::Ports | Self::Lookup | Self::ContainerPorts | Self::ContainerMounts => 32,
            Self::Groups | Self::Containers => 16,
        }
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Ports => tr("shell-live-ports"),
            Self::Lookup => tr("shell-lookup-ports"),
            Self::Groups => tr("shell-project-groups"),
            Self::Containers => tr("shell-containers"),
            Self::ContainerPorts => tr("shell-published-mappings"),
            Self::ContainerMounts => tr("shell-mounts"),
        }
    }
}
pub(crate) fn page_range(
    count: usize,
    requested: usize,
    size: usize,
) -> (std::ops::Range<usize>, usize, usize) {
    let size = size.max(1);
    let pages = count.div_ceil(size).max(1);
    let page = requested.min(pages - 1);
    let start = page * size;
    (start..start.saturating_add(size).min(count), page, pages)
}
