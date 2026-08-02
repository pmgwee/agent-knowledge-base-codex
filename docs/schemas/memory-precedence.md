# Memory precedence and supersession

Retrieval resolves memory in a fixed order:

1. exact project scope;
2. validity at the requested time;
3. explicit version supersession;
4. evidence authority;
5. recording recency;
6. confidence and deterministic ID tie-breaks.

Authority from strongest to weakest is live state, raw mechanical evidence,
human correction, agent checkpoint, derived memory, and external document.
Human corrections therefore outrank agent interpretations for intent and
preferences, while current Git/test/deployment evidence still outranks humans.

Candidates are grouped by memory kind and normalized title. Explicitly
superseded versions remain historical. Lower-authority values remain visible as
historical warnings. Two active values with equal top authority and different
content become an unresolved conflict; the resolver returns both and does not
use vector similarity or recency to invent a winner.
