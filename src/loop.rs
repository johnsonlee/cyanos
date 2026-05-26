//! Two-layer loop orchestration built on the agent abstraction layer.

use std::{fmt, thread};

use crate::{
    agent::{
        AgentOutput, AgentRuntime, AgentRuntimeError, AgentStreamObserver, AgentTurn,
        NoopAgentStreamObserver,
    },
    eval::{Evaluation, Evaluator},
};

/// User intent owned by the outer loop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserIntent {
    value: String,
}

impl UserIntent {
    /// Creates user intent.
    #[must_use]
    pub fn new(value: String) -> Self {
        Self { value }
    }

    /// Returns the user intent.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

/// Prompt used to guide prompt revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OuterPrompt {
    value: String,
}

impl OuterPrompt {
    /// Creates a prompt-revision prompt.
    #[must_use]
    pub fn new(value: String) -> Self {
        Self { value }
    }

    /// Returns the prompt text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

/// Prompt used to guide a coding attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InnerPrompt {
    value: String,
}

impl InnerPrompt {
    /// Creates a coding prompt.
    #[must_use]
    pub fn new(value: String) -> Self {
        Self { value }
    }

    /// Returns the prompt text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

/// Hypothesis generated after evaluation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hypothesis {
    value: String,
}

impl Hypothesis {
    /// Creates a hypothesis.
    #[must_use]
    pub fn new(value: String) -> Self {
        Self { value }
    }

    /// Returns the hypothesis text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

/// Revised prompt produced by the outer loop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptRevision {
    hypothesis: Hypothesis,
    prompt: InnerPrompt,
}

impl PromptRevision {
    /// Creates a prompt revision.
    #[must_use]
    pub fn new(hypothesis: Hypothesis, prompt: InnerPrompt) -> Self {
        Self { hypothesis, prompt }
    }

    /// Returns the hypothesis behind this revision.
    #[must_use]
    pub const fn hypothesis(&self) -> &Hypothesis {
        &self.hypothesis
    }

    /// Returns the revised prompt.
    #[must_use]
    pub const fn prompt(&self) -> &InnerPrompt {
        &self.prompt
    }
}

/// Object that revises the coding prompt after failed evaluation.
pub trait PromptReviser {
    /// Revises the prompt based on evaluation findings.
    #[must_use]
    fn revise(
        &self,
        outer_prompt: &OuterPrompt,
        current: &InnerPrompt,
        evaluation: &Evaluation,
    ) -> PromptRevision;
}

/// Prompt reviser that turns evaluation findings into testable hypotheses.
#[derive(Clone, Copy, Debug)]
pub struct HypothesisPromptReviser {
    action: &'static str,
}

impl Default for HypothesisPromptReviser {
    fn default() -> Self {
        Self {
            action: "avoid this failure in the next code attempt",
        }
    }
}

impl PromptReviser for HypothesisPromptReviser {
    fn revise(
        &self,
        _outer_prompt: &OuterPrompt,
        current: &InnerPrompt,
        evaluation: &Evaluation,
    ) -> PromptRevision {
        let hypothesis = evaluation
            .findings()
            .first()
            .map_or("attempt failed evaluation", |finding| finding.message());
        let revised = format!(
            "{}\n\n## Additional Constraint\n\n- Failure to avoid: {hypothesis}\n- Required adjustment: {}.",
            current.as_str(),
            self.action
        );

        PromptRevision::new(
            Hypothesis::new(hypothesis.to_owned()),
            InnerPrompt::new(revised),
        )
    }
}

/// User-facing task kind used to choose the search topology.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskKind {
    /// Feature work has multiple plausible solutions.
    Feature,
    /// Bug fixes usually search for one correct repair.
    Bugfix,
}

impl TaskKind {
    /// Returns the search shape for this task kind.
    #[must_use]
    pub const fn shape(self) -> TaskShape {
        match self {
            Self::Feature => TaskShape::Multimodal,
            Self::Bugfix => TaskShape::Unimodal,
        }
    }
}

/// Search shape used by the SSD inner loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskShape {
    /// One basin of attraction; preserve each sample's lineage.
    Unimodal,
    /// Multiple valid modes; mix elite preseed with persistent lineages.
    Multimodal,
}

/// Strategy used to seed samples across outer runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineageStrategy {
    /// Each sample keeps its own history across runs.
    PersistentLineages,
    /// The first samples are seeded from the global best; the rest keep lineage.
    LineagesWithElite {
        /// Number of leading samples seeded from global best.
        preseed_count: usize,
    },
}

impl LineageStrategy {
    /// Default number of samples preseeded from the global best for multimodal work.
    pub const DEFAULT_ELITE_PRESEED_COUNT: usize = 1;

    /// Creates the default strategy for a task shape.
    #[must_use]
    pub const fn for_shape(shape: TaskShape) -> Self {
        match shape {
            TaskShape::Unimodal => Self::PersistentLineages,
            TaskShape::Multimodal => Self::LineagesWithElite {
                preseed_count: Self::DEFAULT_ELITE_PRESEED_COUNT,
            },
        }
    }

    /// Returns the sample origin for a one-based sample index.
    #[must_use]
    pub const fn origin_for(self, sample_id: SampleId) -> SampleOrigin {
        match self {
            Self::LineagesWithElite { preseed_count } if sample_id.as_usize() <= preseed_count => {
                SampleOrigin::ElitePreseed
            }
            Self::PersistentLineages | Self::LineagesWithElite { .. } => SampleOrigin::Lineage,
        }
    }
}

/// Error returned when SSD configuration is invalid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SsdConfigError {
    /// At least one sample is required.
    NoSamplesConfigured,
    /// Elite preseed cannot exceed the configured sample count.
    PreseedExceedsSamples,
}

impl fmt::Display for SsdConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSamplesConfigured => f.write_str("no SSD samples configured"),
            Self::PreseedExceedsSamples => {
                f.write_str("elite preseed count exceeds SSD sample count")
            }
        }
    }
}

impl std::error::Error for SsdConfigError {}

/// Configuration for one Sample-Select-Distill inner run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SsdConfig {
    sample_count: usize,
    task_kind: TaskKind,
    elite_preseed_count: usize,
}

impl SsdConfig {
    /// Creates SSD configuration with the default elite preseed policy.
    ///
    /// # Errors
    ///
    /// Returns an error when `sample_count` is zero.
    pub fn new(sample_count: usize, task_kind: TaskKind) -> Result<Self, SsdConfigError> {
        Self::with_elite_preseed_count(
            sample_count,
            task_kind,
            LineageStrategy::DEFAULT_ELITE_PRESEED_COUNT,
        )
    }

    /// Creates SSD configuration with an explicit elite preseed count.
    ///
    /// # Errors
    ///
    /// Returns an error when `sample_count` is zero or when `elite_preseed_count`
    /// exceeds `sample_count`.
    pub fn with_elite_preseed_count(
        sample_count: usize,
        task_kind: TaskKind,
        elite_preseed_count: usize,
    ) -> Result<Self, SsdConfigError> {
        if sample_count == 0 {
            return Err(SsdConfigError::NoSamplesConfigured);
        }

        if elite_preseed_count > sample_count {
            return Err(SsdConfigError::PreseedExceedsSamples);
        }

        Ok(Self {
            sample_count,
            task_kind,
            elite_preseed_count,
        })
    }

    /// Returns the number of samples per SSD run.
    #[must_use]
    pub const fn sample_count(&self) -> usize {
        self.sample_count
    }

    /// Returns the task kind.
    #[must_use]
    pub const fn task_kind(&self) -> TaskKind {
        self.task_kind
    }

    /// Returns the task shape.
    #[must_use]
    pub const fn task_shape(&self) -> TaskShape {
        self.task_kind.shape()
    }

    /// Returns the lineage strategy for this SSD configuration.
    #[must_use]
    pub const fn lineage_strategy(&self) -> LineageStrategy {
        match self.task_shape() {
            TaskShape::Unimodal => LineageStrategy::PersistentLineages,
            TaskShape::Multimodal => LineageStrategy::LineagesWithElite {
                preseed_count: self.elite_preseed_count,
            },
        }
    }
}

/// One-based sample identity inside an SSD run.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SampleId {
    value: usize,
}

impl SampleId {
    const fn from_one_based(value: usize) -> Self {
        Self { value }
    }

    /// Returns the one-based sample index.
    #[must_use]
    pub const fn as_usize(self) -> usize {
        self.value
    }
}

/// Source used to seed a sample at the start of an SSD run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SampleOrigin {
    /// Continue the sample's own history.
    Lineage,
    /// Start from the current global best.
    ElitePreseed,
}

/// Planned independent sample execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SamplePlan {
    id: SampleId,
    origin: SampleOrigin,
    prompt: InnerPrompt,
}

impl SamplePlan {
    /// Creates a sample plan.
    #[must_use]
    pub fn new(id: SampleId, origin: SampleOrigin, prompt: InnerPrompt) -> Self {
        Self { id, origin, prompt }
    }

    /// Returns the sample id.
    #[must_use]
    pub const fn id(&self) -> SampleId {
        self.id
    }

    /// Returns the sample origin.
    #[must_use]
    pub const fn origin(&self) -> SampleOrigin {
        self.origin
    }

    /// Returns the prompt shared by this SSD run.
    #[must_use]
    pub const fn prompt(&self) -> &InnerPrompt {
        &self.prompt
    }
}

/// Planned sample set for one SSD run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SsdRunPlan {
    strategy: LineageStrategy,
    samples: Vec<SamplePlan>,
}

impl SsdRunPlan {
    /// Creates an SSD run plan.
    #[must_use]
    pub fn new(strategy: LineageStrategy, samples: Vec<SamplePlan>) -> Self {
        Self { strategy, samples }
    }

    /// Returns the lineage strategy.
    #[must_use]
    pub const fn strategy(&self) -> LineageStrategy {
        self.strategy
    }

    /// Returns planned samples.
    #[must_use]
    pub fn samples(&self) -> &[SamplePlan] {
        &self.samples
    }

    fn into_samples(self) -> Vec<SamplePlan> {
        self.samples
    }
}

/// Object that plans SSD samples from task shape and lineage policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SamplePlanner {
    config: SsdConfig,
}

impl SamplePlanner {
    /// Creates a sample planner.
    #[must_use]
    pub const fn new(config: SsdConfig) -> Self {
        Self { config }
    }

    /// Creates a plan for the next SSD run.
    #[must_use]
    pub fn plan(&self, inner_prompt: &InnerPrompt) -> SsdRunPlan {
        let strategy = self.config.lineage_strategy();
        let samples = (1..=self.config.sample_count())
            .map(|sample_index| {
                let id = SampleId::from_one_based(sample_index);
                SamplePlan::new(id, strategy.origin_for(id), inner_prompt.clone())
            })
            .collect();

        SsdRunPlan::new(strategy, samples)
    }
}

/// Score assigned to one sample result.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SampleScore {
    value: i64,
}

impl SampleScore {
    /// Creates a sample score.
    #[must_use]
    pub const fn new(value: i64) -> Self {
        Self { value }
    }

    /// Returns the raw score value.
    #[must_use]
    pub const fn as_i64(self) -> i64 {
        self.value
    }
}

/// Scores an evaluated sample.
pub trait SampleScorer {
    /// Scores a sample output after evaluation.
    #[must_use]
    fn score(&self, evaluation: &Evaluation, output: &AgentOutput) -> SampleScore;
}

/// Minimal scorer that ranks accepted samples above rejected samples.
#[derive(Clone, Copy, Debug, Default)]
pub struct VerdictSampleScorer;

impl SampleScorer for VerdictSampleScorer {
    fn score(&self, evaluation: &Evaluation, _output: &AgentOutput) -> SampleScore {
        if evaluation.is_accepted() {
            SampleScore::new(1)
        } else {
            SampleScore::new(0)
        }
    }
}

/// Result of one independent SSD sample.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SampleResult {
    plan: SamplePlan,
    turn: AgentTurn,
    output: AgentOutput,
    evaluation: Evaluation,
    score: SampleScore,
}

impl SampleResult {
    /// Creates a sample result.
    #[must_use]
    pub fn new(
        plan: SamplePlan,
        turn: AgentTurn,
        output: AgentOutput,
        evaluation: Evaluation,
        score: SampleScore,
    ) -> Self {
        Self {
            plan,
            turn,
            output,
            evaluation,
            score,
        }
    }

    /// Returns the sample plan.
    #[must_use]
    pub const fn plan(&self) -> &SamplePlan {
        &self.plan
    }

    /// Returns the agent turn.
    #[must_use]
    pub const fn turn(&self) -> &AgentTurn {
        &self.turn
    }

    /// Returns the sample output.
    #[must_use]
    pub const fn output(&self) -> &AgentOutput {
        &self.output
    }

    /// Returns the sample evaluation.
    #[must_use]
    pub const fn evaluation(&self) -> &Evaluation {
        &self.evaluation
    }

    /// Returns the sample score.
    #[must_use]
    pub const fn score(&self) -> SampleScore {
        self.score
    }
}

/// Object that chooses the highest-scoring sample.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HighestScoreSelector {
    prefer_later_ties: bool,
}

impl HighestScoreSelector {
    /// Creates a score selector.
    #[must_use]
    pub const fn new(prefer_later_ties: bool) -> Self {
        Self { prefer_later_ties }
    }

    /// Selects the highest-scoring sample.
    #[must_use]
    pub fn select(&self, samples: &[SampleResult]) -> Option<SampleResult> {
        if self.prefer_later_ties {
            samples.iter().max_by_key(|sample| sample.score()).cloned()
        } else {
            samples
                .iter()
                .fold(None::<&SampleResult>, |best, sample| match best {
                    Some(best_sample) if best_sample.score() >= sample.score() => Some(best_sample),
                    _ => Some(sample),
                })
                .cloned()
        }
    }
}

/// Completed Sample-Select-Distill inner run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SsdReport {
    strategy: LineageStrategy,
    samples: Vec<SampleResult>,
    global_best: SampleResult,
}

impl SsdReport {
    /// Creates an SSD report.
    #[must_use]
    pub fn new(
        strategy: LineageStrategy,
        samples: Vec<SampleResult>,
        global_best: SampleResult,
    ) -> Self {
        Self {
            strategy,
            samples,
            global_best,
        }
    }

    /// Returns the lineage strategy used for the run.
    #[must_use]
    pub const fn strategy(&self) -> LineageStrategy {
        self.strategy
    }

    /// Returns all sample results.
    #[must_use]
    pub fn samples(&self) -> &[SampleResult] {
        &self.samples
    }

    /// Returns the distilled global best sample.
    #[must_use]
    pub const fn global_best(&self) -> &SampleResult {
        &self.global_best
    }
}

/// Creates independent runtimes for SSD samples.
pub trait AgentRuntimeFactory {
    /// Runtime type created for each sample.
    type Runtime: AgentRuntime + Send;

    /// Creates one runtime for the provided sample plan.
    #[must_use]
    fn runtime_for(&self, sample: &SamplePlan) -> Self::Runtime;
}

/// Creates stream observers for SSD samples.
pub trait SampleObserverFactory {
    /// Creates one observer for the provided sample plan.
    #[must_use]
    fn observer_for(&self, sample: &SamplePlan) -> Box<dyn AgentStreamObserver + Send>;
}

/// Observer factory that ignores all sample stream events.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoopSampleObserverFactory;

impl SampleObserverFactory for NoopSampleObserverFactory {
    fn observer_for(&self, _sample: &SamplePlan) -> Box<dyn AgentStreamObserver + Send> {
        Box::new(NoopAgentStreamObserver)
    }
}

/// Inner Sample-Select-Distill loop.
#[derive(Clone, Debug)]
pub struct SampleSelectDistill<F, E, S, O = NoopSampleObserverFactory> {
    runtime_factory: F,
    evaluator: E,
    scorer: S,
    observer_factory: O,
    planner: SamplePlanner,
    selector: HighestScoreSelector,
}

impl<F, E, S> SampleSelectDistill<F, E, S>
where
    F: AgentRuntimeFactory,
    E: Evaluator + Clone + Send,
    S: SampleScorer + Clone + Send,
{
    /// Creates an inner SSD loop.
    #[must_use]
    pub const fn new(runtime_factory: F, evaluator: E, scorer: S, config: SsdConfig) -> Self {
        Self::with_observer_factory(
            runtime_factory,
            evaluator,
            scorer,
            config,
            NoopSampleObserverFactory,
        )
    }
}

impl<F, E, S, O> SampleSelectDistill<F, E, S, O>
where
    F: AgentRuntimeFactory,
    E: Evaluator + Clone + Send,
    S: SampleScorer + Clone + Send,
    O: SampleObserverFactory + Clone + Send,
{
    /// Creates an inner SSD loop with a sample stream observer factory.
    #[must_use]
    pub const fn with_observer_factory(
        runtime_factory: F,
        evaluator: E,
        scorer: S,
        config: SsdConfig,
        observer_factory: O,
    ) -> Self {
        Self {
            runtime_factory,
            evaluator,
            scorer,
            observer_factory,
            planner: SamplePlanner::new(config),
            selector: HighestScoreSelector::new(false),
        }
    }

    /// Runs all samples independently and distills the highest-scoring result.
    ///
    /// # Errors
    ///
    /// Returns an error when a sample runtime fails or a sample worker cannot
    /// be joined.
    pub fn run(
        &self,
        user_intent: &UserIntent,
        inner_prompt: &InnerPrompt,
    ) -> Result<SsdReport, LoopError> {
        let plan = self.planner.plan(inner_prompt);
        let strategy = plan.strategy();
        let samples = plan.into_samples();
        let mut results = Vec::with_capacity(samples.len());

        thread::scope(|scope| -> Result<(), LoopError> {
            let mut handles = Vec::with_capacity(samples.len());

            for sample in samples {
                let evaluator = self.evaluator.clone();
                let scorer = self.scorer.clone();
                let observer_factory = self.observer_factory.clone();
                let user_intent = user_intent.as_str().to_owned();
                let mut runtime = self.runtime_factory.runtime_for(&sample);

                handles.push(scope.spawn(move || {
                    let turn = AgentTurn::new(sample.prompt().as_str().to_owned(), user_intent);
                    let mut observer = observer_factory.observer_for(&sample);
                    let output = runtime
                        .execute_with_observer(&turn, observer.as_mut())
                        .map_err(LoopError::Agent)?;
                    let evaluation = evaluator.evaluate(&turn, &output);
                    let score = scorer.score(&evaluation, &output);

                    Ok::<SampleResult, LoopError>(SampleResult::new(
                        sample, turn, output, evaluation, score,
                    ))
                }));
            }

            for handle in handles {
                let result = handle
                    .join()
                    .map_err(|_panic| LoopError::SampleWorkerPanicked)??;
                results.push(result);
            }

            Ok(())
        })?;

        let global_best = self
            .selector
            .select(&results)
            .ok_or(LoopError::NoSamplesConfigured)?;

        Ok(SsdReport::new(strategy, results, global_best))
    }
}

/// Configuration for the two-layer loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoopConfig {
    max_attempts: usize,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self { max_attempts: 1 }
    }
}

impl LoopConfig {
    /// Creates loop configuration.
    #[must_use]
    pub const fn new(max_attempts: usize) -> Self {
        Self { max_attempts }
    }

    /// Returns the maximum number of attempts.
    #[must_use]
    pub const fn max_attempts(&self) -> usize {
        self.max_attempts
    }
}

/// Task for the two-layer loop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoopTask {
    user_intent: UserIntent,
    outer_prompt: OuterPrompt,
    inner_prompt: InnerPrompt,
}

impl LoopTask {
    /// Creates a loop task.
    #[must_use]
    pub fn new(
        user_intent: UserIntent,
        outer_prompt: OuterPrompt,
        inner_prompt: InnerPrompt,
    ) -> Self {
        Self {
            user_intent,
            outer_prompt,
            inner_prompt,
        }
    }
}

/// One agent attempt and its evaluation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoopAttempt {
    turn: AgentTurn,
    output: AgentOutput,
    evaluation: Evaluation,
}

impl LoopAttempt {
    /// Creates a loop attempt.
    #[must_use]
    pub fn new(turn: AgentTurn, output: AgentOutput, evaluation: Evaluation) -> Self {
        Self {
            turn,
            output,
            evaluation,
        }
    }

    /// Returns the agent turn.
    #[must_use]
    pub const fn turn(&self) -> &AgentTurn {
        &self.turn
    }

    /// Returns the agent output.
    #[must_use]
    pub const fn output(&self) -> &AgentOutput {
        &self.output
    }

    /// Returns the evaluation.
    #[must_use]
    pub const fn evaluation(&self) -> &Evaluation {
        &self.evaluation
    }
}

/// Final loop status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoopStatus {
    /// Evaluation accepted the output.
    Accepted,
    /// All attempts were exhausted.
    Rejected,
}

/// Report returned after the two-layer loop completes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoopReport {
    status: LoopStatus,
    attempts: Vec<LoopAttempt>,
    final_prompt: InnerPrompt,
}

impl LoopReport {
    /// Creates a loop report.
    #[must_use]
    pub fn new(status: LoopStatus, attempts: Vec<LoopAttempt>, final_prompt: InnerPrompt) -> Self {
        Self {
            status,
            attempts,
            final_prompt,
        }
    }

    /// Returns the final status.
    #[must_use]
    pub const fn status(&self) -> LoopStatus {
        self.status
    }

    /// Returns all attempts.
    #[must_use]
    pub fn attempts(&self) -> &[LoopAttempt] {
        &self.attempts
    }

    /// Returns the final coding prompt.
    #[must_use]
    pub const fn final_prompt(&self) -> &InnerPrompt {
        &self.final_prompt
    }
}

/// Error returned by the loop engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoopError {
    /// Configuration does not allow any attempts.
    NoAttemptsConfigured,
    /// SSD configuration does not allow any samples.
    NoSamplesConfigured,
    /// SSD configuration is invalid.
    SsdConfig(SsdConfigError),
    /// Agent attempt execution failed.
    Agent(AgentRuntimeError),
    /// A parallel SSD sample worker failed to join.
    SampleWorkerPanicked,
}

impl fmt::Display for LoopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAttemptsConfigured => f.write_str("no loop attempts configured"),
            Self::NoSamplesConfigured => f.write_str("no SSD samples configured"),
            Self::SsdConfig(error) => write!(f, "invalid SSD configuration: {error}"),
            Self::Agent(error) => write!(f, "agent attempt failed: {error}"),
            Self::SampleWorkerPanicked => f.write_str("SSD sample worker panicked"),
        }
    }
}

impl std::error::Error for LoopError {}

impl From<SsdConfigError> for LoopError {
    fn from(error: SsdConfigError) -> Self {
        Self::SsdConfig(error)
    }
}

/// Report returned by the nested outer loop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NestedLoopReport {
    status: LoopStatus,
    runs: Vec<SsdReport>,
    final_prompt: InnerPrompt,
}

impl NestedLoopReport {
    /// Creates a nested loop report.
    #[must_use]
    pub fn new(status: LoopStatus, runs: Vec<SsdReport>, final_prompt: InnerPrompt) -> Self {
        Self {
            status,
            runs,
            final_prompt,
        }
    }

    /// Returns the final status.
    #[must_use]
    pub const fn status(&self) -> LoopStatus {
        self.status
    }

    /// Returns all SSD runs.
    #[must_use]
    pub fn runs(&self) -> &[SsdReport] {
        &self.runs
    }

    /// Returns the final coding prompt.
    #[must_use]
    pub const fn final_prompt(&self) -> &InnerPrompt {
        &self.final_prompt
    }
}

/// Outer prompt-evolution loop that wraps the inner SSD loop.
#[derive(Clone, Debug)]
pub struct NestedLoop<F, E, S, P> {
    inner_loop: SampleSelectDistill<F, E, S>,
    prompt_reviser: P,
    config: LoopConfig,
}

impl<F, E, S, P> NestedLoop<F, E, S, P>
where
    F: AgentRuntimeFactory,
    E: Evaluator + Clone + Send,
    S: SampleScorer + Clone + Send,
    P: PromptReviser,
{
    /// Creates a nested two-layer loop.
    #[must_use]
    pub fn new(
        inner_loop: SampleSelectDistill<F, E, S>,
        prompt_reviser: P,
        config: LoopConfig,
    ) -> Self {
        Self {
            inner_loop,
            prompt_reviser,
            config,
        }
    }

    /// Runs prompt evolution over repeated SSD runs.
    ///
    /// # Errors
    ///
    /// Returns an error when no outer runs are configured or when an SSD run
    /// fails.
    pub fn run(&self, task: LoopTask) -> Result<NestedLoopReport, LoopError> {
        if self.config.max_attempts() == 0 {
            return Err(LoopError::NoAttemptsConfigured);
        }

        let mut prompt = task.inner_prompt;
        let mut runs = Vec::new();

        for _ in 0..self.config.max_attempts() {
            let report = self.inner_loop.run(&task.user_intent, &prompt)?;
            let best_evaluation = report.global_best().evaluation().clone();
            let accepted = best_evaluation.is_accepted();

            runs.push(report);

            if accepted {
                return Ok(NestedLoopReport::new(LoopStatus::Accepted, runs, prompt));
            }

            let revision =
                self.prompt_reviser
                    .revise(&task.outer_prompt, &prompt, &best_evaluation);
            prompt = revision.prompt().clone();
        }

        Ok(NestedLoopReport::new(LoopStatus::Rejected, runs, prompt))
    }
}

/// Loop object that guides, evaluates, and revises agent attempts.
#[derive(Clone, Debug)]
pub struct TwoLayerLoop<R, E, P> {
    inner_agent: R,
    evaluator: E,
    prompt_reviser: P,
    config: LoopConfig,
}

impl<R, E, P> TwoLayerLoop<R, E, P>
where
    R: AgentRuntime,
    E: Evaluator,
    P: PromptReviser,
{
    /// Creates a two-layer loop.
    #[must_use]
    pub fn new(inner_agent: R, evaluator: E, prompt_reviser: P, config: LoopConfig) -> Self {
        Self {
            inner_agent,
            evaluator,
            prompt_reviser,
            config,
        }
    }

    /// Runs the loop against the agent.
    ///
    /// # Errors
    ///
    /// Returns an error when no attempts are configured or the agent
    /// runtime fails.
    pub fn run(&mut self, task: LoopTask) -> Result<LoopReport, LoopError> {
        if self.config.max_attempts() == 0 {
            return Err(LoopError::NoAttemptsConfigured);
        }

        let mut prompt = task.inner_prompt;
        let mut attempts = Vec::new();

        for _ in 0..self.config.max_attempts() {
            let turn = AgentTurn::new(
                prompt.as_str().to_owned(),
                task.user_intent.as_str().to_owned(),
            );
            let output = self.inner_agent.execute(&turn).map_err(LoopError::Agent)?;
            let evaluation = self.evaluator.evaluate(&turn, &output);
            let accepted = evaluation.is_accepted();

            attempts.push(LoopAttempt::new(turn, output, evaluation.clone()));

            if accepted {
                return Ok(LoopReport::new(LoopStatus::Accepted, attempts, prompt));
            }

            let revision = self
                .prompt_reviser
                .revise(&task.outer_prompt, &prompt, &evaluation);
            prompt = revision.prompt().clone();
        }

        Ok(LoopReport::new(LoopStatus::Rejected, attempts, prompt))
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        agent::{AgentOutput, AgentRuntime, AgentRuntimeError, AgentTurn},
        eval::{Evaluation, EvaluationFinding, NonEmptyOutputEvaluator},
        r#loop::{
            AgentRuntimeFactory, HypothesisPromptReviser, InnerPrompt, LineageStrategy,
            LoopAttempt, LoopConfig, LoopError, LoopReport, LoopStatus, LoopTask, NestedLoop,
            NestedLoopReport, OuterPrompt, PromptReviser, PromptRevision, SampleOrigin, SamplePlan,
            SampleResult, SampleScore, SampleScorer, SampleSelectDistill, SsdConfig,
            SsdConfigError, SsdReport, TaskKind, TaskShape, TwoLayerLoop, UserIntent,
            VerdictSampleScorer,
        },
    };

    #[derive(Debug)]
    struct ScriptedRuntime {
        outputs: Vec<String>,
        calls: usize,
    }

    impl ScriptedRuntime {
        fn new(outputs: Vec<String>) -> Self {
            Self { outputs, calls: 0 }
        }
    }

    impl AgentRuntime for ScriptedRuntime {
        fn execute(&mut self, _turn: &AgentTurn) -> Result<AgentOutput, AgentRuntimeError> {
            let output = self.outputs.get(self.calls).cloned().unwrap_or_default();
            self.calls += 1;
            Ok(AgentOutput::new(output))
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct SampleRuntimeFactory;

    impl AgentRuntimeFactory for SampleRuntimeFactory {
        type Runtime = SampleRuntime;

        fn runtime_for(&self, sample: &SamplePlan) -> Self::Runtime {
            SampleRuntime::new(sample.id().as_usize())
        }
    }

    #[derive(Debug)]
    struct SampleRuntime {
        sample_id: usize,
    }

    impl SampleRuntime {
        const fn new(sample_id: usize) -> Self {
            Self { sample_id }
        }
    }

    impl AgentRuntime for SampleRuntime {
        fn execute(&mut self, _turn: &AgentTurn) -> Result<AgentOutput, AgentRuntimeError> {
            let output = match self.sample_id {
                1 => "short",
                2 => "longer global best",
                _ => "",
            };

            Ok(AgentOutput::new(output.to_owned()))
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct ContentLengthScorer;

    impl SampleScorer for ContentLengthScorer {
        fn score(&self, _evaluation: &Evaluation, output: &AgentOutput) -> SampleScore {
            i64::try_from(output.content().len())
                .map_or_else(|_| SampleScore::new(i64::MAX), SampleScore::new)
        }
    }

    #[test]
    fn accepts_output_that_passes_eval() -> Result<(), LoopError> {
        let runtime = ScriptedRuntime::new(vec!["done".to_owned()]);
        let mut outer_loop = TwoLayerLoop::new(
            runtime,
            NonEmptyOutputEvaluator::default(),
            HypothesisPromptReviser::default(),
            LoopConfig::new(2),
        );
        let report = outer_loop.run(LoopTask::new(
            UserIntent::new("build a cli".to_owned()),
            OuterPrompt::new("guide and evaluate attempt".to_owned()),
            InnerPrompt::new("execute faithfully".to_owned()),
        ))?;

        assert_eq!(report.status(), LoopStatus::Accepted);
        assert_eq!(report.attempts().len(), 1);

        Ok(())
    }

    #[test]
    fn revises_prompt_after_failed_eval() -> Result<(), LoopError> {
        let runtime = ScriptedRuntime::new(vec![String::new(), "done".to_owned()]);
        let mut outer_loop = TwoLayerLoop::new(
            runtime,
            NonEmptyOutputEvaluator::default(),
            HypothesisPromptReviser::default(),
            LoopConfig::new(2),
        );
        let report = outer_loop.run(LoopTask::new(
            UserIntent::new("build a cli".to_owned()),
            OuterPrompt::new("guide and evaluate attempt".to_owned()),
            InnerPrompt::new("execute faithfully".to_owned()),
        ))?;

        assert_eq!(report.status(), LoopStatus::Accepted);
        assert_eq!(report.attempts().len(), 2);
        assert!(report.final_prompt().as_str().contains("Failure to avoid:"));

        Ok(())
    }

    #[test]
    fn rejects_zero_attempt_configuration() {
        let runtime = ScriptedRuntime::new(vec!["done".to_owned()]);
        let mut outer_loop = TwoLayerLoop::new(
            runtime,
            NonEmptyOutputEvaluator::default(),
            HypothesisPromptReviser::default(),
            LoopConfig::new(0),
        );
        let result = outer_loop.run(LoopTask::new(
            UserIntent::new("build a cli".to_owned()),
            OuterPrompt::new("guide and evaluate attempt".to_owned()),
            InnerPrompt::new("execute faithfully".to_owned()),
        ));

        assert_eq!(result, Err(LoopError::NoAttemptsConfigured));
    }

    #[test]
    fn exposes_prompt_hypothesis_revision_and_attempt_fields() {
        let user_intent = UserIntent::new("intent".to_owned());
        let outer_prompt = OuterPrompt::new("outer".to_owned());
        let inner_prompt = InnerPrompt::new("inner".to_owned());
        let hypothesis = super::Hypothesis::new("reason".to_owned());
        let revision = PromptRevision::new(hypothesis.clone(), inner_prompt.clone());
        let turn = AgentTurn::new("system".to_owned(), "user".to_owned());
        let output = AgentOutput::new("content".to_owned());
        let evaluation = Evaluation::accepted();
        let attempt = LoopAttempt::new(turn.clone(), output.clone(), evaluation.clone());

        assert_eq!(user_intent.as_str(), "intent");
        assert_eq!(outer_prompt.as_str(), "outer");
        assert_eq!(inner_prompt.as_str(), "inner");
        assert_eq!(hypothesis.as_str(), "reason");
        assert_eq!(revision.hypothesis().as_str(), "reason");
        assert_eq!(revision.prompt().as_str(), "inner");
        assert_eq!(attempt.turn(), &turn);
        assert_eq!(attempt.output(), &output);
        assert_eq!(attempt.evaluation(), &evaluation);
    }

    #[test]
    fn exposes_config_strategy_and_error_values() {
        assert_eq!(LoopConfig::default().max_attempts(), 1);
        assert_eq!(TaskKind::Feature.shape(), TaskShape::Multimodal);
        assert_eq!(TaskKind::Bugfix.shape(), TaskShape::Unimodal);
        assert_eq!(
            LineageStrategy::for_shape(TaskShape::Unimodal),
            LineageStrategy::PersistentLineages
        );
        assert_eq!(
            LineageStrategy::for_shape(TaskShape::Multimodal),
            LineageStrategy::LineagesWithElite { preseed_count: 1 }
        );
        assert_eq!(
            LineageStrategy::LineagesWithElite { preseed_count: 2 }
                .origin_for(super::SampleId::from_one_based(2)),
            SampleOrigin::ElitePreseed
        );
        assert_eq!(
            SsdConfig::new(0, TaskKind::Feature),
            Err(SsdConfigError::NoSamplesConfigured)
        );
        assert_eq!(
            SsdConfig::with_elite_preseed_count(1, TaskKind::Feature, 2),
            Err(SsdConfigError::PreseedExceedsSamples)
        );
        assert_eq!(
            SsdConfigError::NoSamplesConfigured.to_string(),
            "no SSD samples configured"
        );
        assert_eq!(
            SsdConfigError::PreseedExceedsSamples.to_string(),
            "elite preseed count exceeds SSD sample count"
        );
    }

    #[test]
    fn plans_persistent_lineages_for_bugfix() -> Result<(), LoopError> {
        let config = SsdConfig::new(3, TaskKind::Bugfix)?;
        let plan =
            super::SamplePlanner::new(config).plan(&InnerPrompt::new("fix the bug".to_owned()));
        let origins: Vec<_> = plan.samples().iter().map(SamplePlan::origin).collect();

        assert_eq!(plan.strategy(), LineageStrategy::PersistentLineages);
        assert_eq!(
            origins,
            [
                super::SampleOrigin::Lineage,
                super::SampleOrigin::Lineage,
                super::SampleOrigin::Lineage
            ]
        );

        Ok(())
    }

    #[test]
    fn plans_elite_preseed_for_feature() -> Result<(), LoopError> {
        let config = SsdConfig::new(3, TaskKind::Feature)?;
        assert_eq!(config.task_kind(), TaskKind::Feature);
        assert_eq!(config.task_shape(), TaskShape::Multimodal);
        let plan =
            super::SamplePlanner::new(config).plan(&InnerPrompt::new("build feature".to_owned()));
        let origins: Vec<_> = plan.samples().iter().map(SamplePlan::origin).collect();

        assert_eq!(
            plan.strategy(),
            LineageStrategy::LineagesWithElite { preseed_count: 1 }
        );
        assert_eq!(
            origins,
            [
                super::SampleOrigin::ElitePreseed,
                super::SampleOrigin::Lineage,
                super::SampleOrigin::Lineage
            ]
        );

        Ok(())
    }

    #[test]
    fn exposes_sample_result_and_ssd_report_fields() {
        let plan = SamplePlan::new(
            super::SampleId::from_one_based(1),
            SampleOrigin::Lineage,
            InnerPrompt::new("prompt".to_owned()),
        );
        let turn = AgentTurn::new("prompt".to_owned(), "intent".to_owned());
        let output = AgentOutput::new("output".to_owned());
        let evaluation = Evaluation::accepted();
        let result = SampleResult::new(
            plan.clone(),
            turn.clone(),
            output.clone(),
            evaluation.clone(),
            SampleScore::new(7),
        );
        let report = SsdReport::new(
            LineageStrategy::PersistentLineages,
            vec![result.clone()],
            result.clone(),
        );

        assert_eq!(plan.id().as_usize(), 1);
        assert_eq!(plan.origin(), SampleOrigin::Lineage);
        assert_eq!(plan.prompt().as_str(), "prompt");
        assert_eq!(result.plan(), &plan);
        assert_eq!(result.turn(), &turn);
        assert_eq!(result.output(), &output);
        assert_eq!(result.evaluation(), &evaluation);
        assert_eq!(result.score().as_i64(), 7);
        assert_eq!(report.strategy(), LineageStrategy::PersistentLineages);
        assert_eq!(report.samples(), std::slice::from_ref(&result));
        assert_eq!(report.global_best(), &result);
    }

    #[test]
    fn nested_loop_distills_highest_scoring_sample() -> Result<(), LoopError> {
        let ssd = SampleSelectDistill::new(
            SampleRuntimeFactory,
            NonEmptyOutputEvaluator::default(),
            ContentLengthScorer,
            SsdConfig::new(3, TaskKind::Feature)?,
        );
        let nested_loop =
            NestedLoop::new(ssd, HypothesisPromptReviser::default(), LoopConfig::new(2));
        let report = nested_loop.run(LoopTask::new(
            UserIntent::new("build a feature".to_owned()),
            OuterPrompt::new("evaluate SSD result".to_owned()),
            InnerPrompt::new("attempt implementation".to_owned()),
        ))?;

        let best_id = report
            .runs()
            .first()
            .map(|run| run.global_best().plan().id().as_usize());

        assert_eq!(report.status(), LoopStatus::Accepted);
        assert_eq!(report.runs().len(), 1);
        assert_eq!(best_id, Some(2));

        Ok(())
    }

    #[test]
    fn nested_loop_report_exposes_fields() {
        let prompt = InnerPrompt::new("prompt".to_owned());
        let report = NestedLoopReport::new(LoopStatus::Rejected, Vec::new(), prompt.clone());

        assert_eq!(report.status(), LoopStatus::Rejected);
        assert!(report.runs().is_empty());
        assert_eq!(report.final_prompt(), &prompt);
    }

    #[test]
    fn nested_loop_revises_prompt_after_rejected_global_best() -> Result<(), LoopError> {
        let ssd = SampleSelectDistill::new(
            ScriptedRuntimeFactory,
            NonEmptyOutputEvaluator::default(),
            VerdictSampleScorer,
            SsdConfig::new(1, TaskKind::Bugfix)?,
        );
        let nested_loop =
            NestedLoop::new(ssd, HypothesisPromptReviser::default(), LoopConfig::new(1));
        let report = nested_loop.run(LoopTask::new(
            UserIntent::new("fix a bug".to_owned()),
            OuterPrompt::new("evaluate SSD result".to_owned()),
            InnerPrompt::new("attempt implementation".to_owned()),
        ))?;

        assert_eq!(report.status(), LoopStatus::Rejected);
        assert!(report.final_prompt().as_str().contains("Failure to avoid:"));

        Ok(())
    }

    #[test]
    fn nested_loop_rejects_zero_outer_runs() -> Result<(), LoopError> {
        let ssd = SampleSelectDistill::new(
            ScriptedRuntimeFactory,
            NonEmptyOutputEvaluator::default(),
            VerdictSampleScorer,
            SsdConfig::new(1, TaskKind::Bugfix)?,
        );
        let nested_loop =
            NestedLoop::new(ssd, HypothesisPromptReviser::default(), LoopConfig::new(0));
        let result = nested_loop.run(LoopTask::new(
            UserIntent::new("fix a bug".to_owned()),
            OuterPrompt::new("evaluate SSD result".to_owned()),
            InnerPrompt::new("attempt implementation".to_owned()),
        ));

        assert_eq!(result, Err(LoopError::NoAttemptsConfigured));
        Ok(())
    }

    #[test]
    fn two_layer_loop_propagates_agent_errors() {
        let runtime = FailingRuntime;
        let mut outer_loop = TwoLayerLoop::new(
            runtime,
            NonEmptyOutputEvaluator::default(),
            HypothesisPromptReviser::default(),
            LoopConfig::new(1),
        );
        let result = outer_loop.run(LoopTask::new(
            UserIntent::new("build a cli".to_owned()),
            OuterPrompt::new("guide and evaluate attempt".to_owned()),
            InnerPrompt::new("execute faithfully".to_owned()),
        ));

        assert_eq!(
            result,
            Err(LoopError::Agent(AgentRuntimeError::new(
                "failed".to_owned()
            )))
        );
        assert_eq!(
            LoopError::Agent(AgentRuntimeError::new("failed".to_owned())).to_string(),
            "agent attempt failed: failed"
        );
        assert_eq!(
            LoopError::SsdConfig(SsdConfigError::NoSamplesConfigured).to_string(),
            "invalid SSD configuration: no SSD samples configured"
        );
        assert_eq!(
            LoopError::SampleWorkerPanicked.to_string(),
            "SSD sample worker panicked"
        );
    }

    #[test]
    fn prompt_reviser_uses_default_hypothesis_when_findings_are_empty() {
        let reviser = HypothesisPromptReviser::default();
        let revision = reviser.revise(
            &OuterPrompt::new("outer".to_owned()),
            &InnerPrompt::new("inner".to_owned()),
            &Evaluation::rejected(Vec::new()),
        );

        assert_eq!(revision.hypothesis().as_str(), "attempt failed evaluation");
        assert!(revision.prompt().as_str().contains("Additional Constraint"));
    }

    #[test]
    fn prompt_reviser_uses_first_evaluation_finding() {
        let reviser = HypothesisPromptReviser::default();
        let revision = reviser.revise(
            &OuterPrompt::new("outer".to_owned()),
            &InnerPrompt::new("inner".to_owned()),
            &Evaluation::rejected(vec![EvaluationFinding::new("specific".to_owned())]),
        );

        assert_eq!(revision.hypothesis().as_str(), "specific");
    }

    #[test]
    fn loop_report_exposes_fields() {
        let prompt = InnerPrompt::new("prompt".to_owned());
        let report = LoopReport::new(LoopStatus::Accepted, Vec::new(), prompt.clone());

        assert_eq!(report.status(), LoopStatus::Accepted);
        assert!(report.attempts().is_empty());
        assert_eq!(report.final_prompt(), &prompt);
    }

    #[derive(Debug)]
    struct FailingRuntime;

    impl AgentRuntime for FailingRuntime {
        fn execute(&mut self, _turn: &AgentTurn) -> Result<AgentOutput, AgentRuntimeError> {
            Err(AgentRuntimeError::new("failed".to_owned()))
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct ScriptedRuntimeFactory;

    impl AgentRuntimeFactory for ScriptedRuntimeFactory {
        type Runtime = ScriptedRuntime;

        fn runtime_for(&self, _sample: &SamplePlan) -> Self::Runtime {
            ScriptedRuntime::new(vec![String::new()])
        }
    }
}
