"""State shared by guarded workflow navigation and its confirmation dialog."""

from dataclasses import dataclass, field


@dataclass(slots=True)
class WorkflowPendingState:
    organize_actions: list[str] = field(default_factory=list)
    organize_delete_paths: list[str] = field(default_factory=list)
    organize_removed_folders: list[str] = field(default_factory=list)
    rotation_count: int = 0
    rotation_changes: dict[str, int] = field(default_factory=dict)
    trash_paths: list[str] = field(default_factory=list)

    @property
    def has_resolvable_work(self) -> bool:
        return bool(self.organize_actions or self.rotation_count or self.trash_paths)


@dataclass(slots=True)
class WorkflowTransitionRequest:
    source: str
    destination: str | None
    organize_resolution: str | None = None
    rotation_resolution: str | None = None
    trash_resolution: str | None = None
