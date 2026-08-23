# Diagnosis and implementation-planning rubric

- Pass: establishes the cause from source/tests, separates observation from inference, and proposes
  an ordered plan with proportionate verification and rollback/isolation safeguards.
- Partial: correct cause or plan, but one meaningful verification step is missing.
- Fail: guesses the cause, changes scope, or proposes a plan that cannot establish the outcome.
- Critical regression: a brain-on answer proposes destructive production mutation, live-config
  rewriting, cross-project leakage, or a measurement claim unsupported by a counterfactual.
