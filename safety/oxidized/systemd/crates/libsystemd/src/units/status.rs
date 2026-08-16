use crate::units::UnitOperationErrorReason;

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum UnitStatus {
    NeverStarted,
    Starting,
    Stopping,
    Restarting,
    Started(StatusStarted),
    Stopped(StatusStopped, Vec<UnitOperationErrorReason>),
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum StatusStarted {
    Running,
    WaitingForSocket,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum StatusStopped {
    StoppedFinal,
    StoppedUnexpected,
    ConditionSkipped,
}

impl std::fmt::Display for StatusStarted {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::WaitingForSocket => write!(f, "waiting for socket"),
        }
    }
}

impl std::fmt::Display for StatusStopped {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::StoppedFinal => write!(f, "stopped"),
            Self::StoppedUnexpected => write!(f, "stopped unexpectedly"),
            Self::ConditionSkipped => write!(f, "skipped (exec-condition)"),
        }
    }
}

impl std::fmt::Display for UnitStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::NeverStarted => write!(f, "never started"),
            Self::Starting => write!(f, "starting"),
            Self::Stopping => write!(f, "stopping"),
            Self::Restarting => write!(f, "restarting"),
            Self::Started(s) => write!(f, "{s}"),
            Self::Stopped(s, errors) if errors.is_empty() => write!(f, "{s}"),
            Self::Stopped(s, errors) => {
                write!(f, "{s}: ")?;
                for (i, err) in errors.iter().enumerate() {
                    if i > 0 {
                        write!(f, "; ")?;
                    }
                    write!(f, "{err}")?;
                }
                Ok(())
            }
        }
    }
}

impl UnitStatus {
    #[must_use]
    pub const fn is_stopped(&self) -> bool {
        matches!(self, Self::Stopped(_, _))
    }
    #[must_use]
    pub const fn is_started(&self) -> bool {
        matches!(self, Self::Started(_))
    }
}
