# PhotoSort Development Guidelines

## Feature design

Before implementing a feature, inspect the complete relevant execution path and
search every workflow for equivalent behavior. Trace the request through the UI,
controllers, application state, workers, services, caches, models, and filesystem
operations. Do not implement a page-local solution until existing shared behavior
has been identified.

PhotoSort's Organize, Easy Delete, Fix Rotation, Pick Best, and Cull workflows
must share application-level capabilities. Thumbnail and preview generation,
image decoding, metadata loading, model initialization, caching, file mutation,
and background-worker lifecycle must not be independently reimplemented by each
workflow.

When adding or changing behavior:

- Reuse or extend an existing implementation when its responsibility matches.
- Consolidate genuine duplication into one clearly owned service, controller,
  component, utility, or worker.
- Keep workflow pages focused on presentation and workflow-specific configuration.
- Maintain one source of truth for shared state and business rules.
- Reuse the shared image pipeline, caches, application state, file-operation
  service, workers, and reusable UI components.
- Use dependency injection instead of constructing competing pipelines, caches,
  services, or model instances.
- Cache reusable results and invalidate them centrally after file mutations.
- Keep expensive work off the UI thread, make it cancellable, and discard stale
  results when the user moves on.
- Do not copy code and make small workflow-specific variations. Prefer parameters
  or focused strategy objects when behavior genuinely varies.
- Do not introduce an abstraction for code used only once unless it materially
  improves ownership, testing, or dependency boundaries.
- Preserve existing behavior unless the feature explicitly changes it.

## Verification

Add tests at the shared ownership boundary, not only at individual pages. When a
feature reuses expensive work, include a regression test proving that scanning,
decoding, thumbnail generation, preview generation, metadata loading, or model
initialization is not performed redundantly.

Before considering a change complete:

- Confirm all affected workflows use the shared implementation.
- Document any intentional duplication and why it remains preferable.
- Run focused tests for the changed behavior and the complete test suite.
- Check UI responsiveness and cancellation behavior for background processing.
