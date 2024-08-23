use number_generics::{Number, One};
use std::{
    marker::PhantomData,
    sync::{Arc, LazyLock, Mutex},
};

static CURRENT_STEP: LazyLock<Mutex<Option<Arc<Step>>>> = LazyLock::new(|| Mutex::new(None));

pub fn current() -> Option<Arc<Step>> {
    CURRENT_STEP.lock().unwrap().clone()
}

pub struct Step {
    pub parent: Option<Arc<Step>>,
    pub number: usize,
    pub total: usize,
}

pub struct StepHandle<Preceding, Remaining> {
    marker: PhantomData<(Preceding, Remaining)>,
    step: Arc<Step>,
}

impl<Preceding: Number, Remaining: Number> StepHandle<Preceding, Remaining> {
    fn new() -> Self {
        let mut current_step_guard = CURRENT_STEP.lock().unwrap();
        let step = Arc::new(Step {
            parent: current_step_guard.take(),
            number: Preceding::len() + 1,
            total: Preceding::len() + 1 + Remaining::len(),
        });
        *current_step_guard = Some(step.clone());
        Self { marker: PhantomData, step }
    }

    // fn contains(&self, other: &)
}

impl<Preceding, Remaining> Drop for StepHandle<Preceding, Remaining> {
    fn drop(&mut self) {
        let mut current_step_guard = CURRENT_STEP.lock().unwrap();
        let current_step = current_step_guard.take().expect("a current step");
        assert!(
            Arc::ptr_eq(&current_step, &self.step),
            "current step is not the dropping one - overlapping intervals?"
        );
        *current_step_guard = current_step.parent.clone();
    }
}

// Non-final Step
impl<Preceding: Number, Remaining: Number> StepHandle<Preceding, One<Remaining>> {
    pub fn next(self) -> StepHandle<One<Preceding>, Remaining> {
        drop(self);
        StepHandle::new()
    }
}

pub fn start<Remaining: Number>() -> StepHandle<(), Remaining> {
    StepHandle::new()
}

// This needs to be a function, not an associated method, so it can contraint the inference to 0 following steps.
pub fn end<Preceding>(_: StepHandle<Preceding, ()>) {}

mod number_generics {
    use std::marker::PhantomData;

    pub trait Number {
        fn len() -> usize;
    }

    pub struct One<T>(PhantomData<T>);

    impl<T: Number> Number for One<T> {
        fn len() -> usize {
            T::len() + 1
        }
    }
    impl Number for () {
        fn len() -> usize {
            0
        }
    }
}

pub mod fmt {
    use super::*;
    use std::fmt::{Display, Formatter};

    pub trait StepExt {
        fn log_prefix(self) -> StepLogPrefix;
    }

    impl StepExt for Option<Arc<Step>> {
        fn log_prefix(self) -> StepLogPrefix {
            StepLogPrefix(self)
        }
    }

    pub struct StepLogPrefix(Option<Arc<Step>>);
    impl Display for StepLogPrefix {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            write_log_prefix(f, &self.0)
        }
    }

    /// Writes the prefix representing one `Step`.
    fn write_step_prefix(f: &mut Formatter, step: &Arc<Step>, visible: bool) -> std::fmt::Result {
        let total_width = step.total.to_string().len();
        if visible {
            write!(f, "[{:total_width$}/{:total_width$}] ", step.number, step.total)
        } else {
            write!(f, " {:total_width$} {:total_width$}  ", "", "")
        }
    }

    /// Writes the representing all `Step`'s. Ancestors are never visible, last one depends on `visible`.
    fn write_steps_prefix(f: &mut Formatter, step: &Option<Arc<Step>>, visible: bool) -> std::fmt::Result {
        if let Some(step) = step {
            write_steps_prefix(f, &step.parent, false)?;
            write_step_prefix(f, step, visible)?;
        }
        Ok(())
    }

    static PREVIOUS_STEP: LazyLock<Mutex<Option<Arc<Step>>>> = LazyLock::new(|| Mutex::new(None));

    /// Writes the log prefix. Deduplicates across lines.
    fn write_log_prefix(f: &mut Formatter, current_step: &Option<Arc<Step>>) -> std::fmt::Result {
        let Some(current_step) = current_step else { return Ok(()) };
        let mut previous_step_guard = PREVIOUS_STEP.lock().unwrap();

        // If current step matches previous at some level just indent, otherwise indent and show.
        let mut previous_step = &*previous_step_guard;
        let visible = loop {
            match previous_step {
                Some(p) if Arc::ptr_eq(current_step, &p) => break false,
                Some(p) => previous_step = &p.parent,
                None => break true,
            }
        };
        *previous_step_guard = Some(current_step.clone());
        write_steps_prefix(f, &Some(current_step.clone()), visible)
    }
}
