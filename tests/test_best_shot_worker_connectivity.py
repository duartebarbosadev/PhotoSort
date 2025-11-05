import pytest

from workers.best_shot_worker import BestShotWorker
from core.ai.best_shot_pipeline import BaseBestShotStrategy, BestShotEngine


class _FailingStrategy(BaseBestShotStrategy):
    def __init__(self):
        super().__init__(models_root=None, image_pipeline=None, llm_config=None)

    def rank_cluster(self, cluster_id, image_paths):
        return []

    def rate_image(self, image_path):
        return None

    def validate_connection(self):
        raise RuntimeError(
            "Unable to reach LLM endpoint at http://localhost:8000: Connection refused"
        )


def test_best_shot_worker_reports_connectivity_issue_during_initialisation():
    worker = BestShotWorker(
        cluster_map={},
        strategy=_FailingStrategy(),
        engine=BestShotEngine.LLM.value,
    )

    with pytest.raises(RuntimeError) as excinfo:
        worker._ensure_strategy()

    message = str(excinfo.value)
    assert "AI service is unreachable" in message
    assert "Details:" in message


def test_best_shot_worker_reports_connectivity_issue_during_processing():
    worker = BestShotWorker(
        cluster_map={},
        engine=BestShotEngine.LLM.value,
    )

    message = worker._format_cluster_error(
        RuntimeError("Connection reset by peer"), cluster_id=7
    )

    assert "AI service became unreachable" in message
    assert "cluster 7" in message.lower()


def test_best_shot_worker_cluster_error_generic_message():
    worker = BestShotWorker(
        cluster_map={},
        engine=BestShotEngine.LLM.value,
    )

    message = worker._format_cluster_error(
        RuntimeError("Unexpected processing failure"), cluster_id=3
    )

    assert message == "Cluster 3 ranking failed: Unexpected processing failure"
