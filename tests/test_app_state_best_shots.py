from unittest.mock import patch

from ui.app_state import AppState


def test_best_shot_results_set_and_cleanup():
    with patch("ui.app_state.RatingCache"), patch("ui.app_state.ExifCache"):
        state = AppState()
        state.cluster_results = {"a.jpg": 1, "b.jpg": 1}

        rankings = {
            1: [
                {
                    "image_path": "a.jpg",
                    "composite_score": 0.9,
                    "metrics": {"aesthetic": 0.8},
                },
                {
                    "image_path": "b.jpg",
                    "composite_score": 0.7,
                    "metrics": {"technical": 0.6},
                },
            ]
        }

        state.set_best_shot_results(rankings)
        assert state.best_shot_winners[1]["image_path"] == "a.jpg"
        assert state.best_shot_scores_by_path["b.jpg"]["composite_score"] == 0.7

        # Renaming propagates to best shot caches
        state.update_path("a.jpg", "c.jpg")
        assert state.best_shot_winners[1]["image_path"] == "c.jpg"
        assert "c.jpg" in state.best_shot_scores_by_path
        assert "a.jpg" not in state.best_shot_scores_by_path

        # Removing the winning path clears winner data
        state.remove_data_for_path("c.jpg")
        assert 1 not in state.best_shot_winners
        assert "c.jpg" not in state.best_shot_scores_by_path


def test_best_shot_results_selection_cluster_fallback():
    with patch("ui.app_state.RatingCache"), patch("ui.app_state.ExifCache"):
        state = AppState()

        rankings = {
            0: [
                {
                    "image_path": "sel1.jpg",
                    "composite_score": 0.95,
                    "metrics": {"aesthetic": 0.9},
                },
                {
                    "image_path": "sel2.jpg",
                    "composite_score": 0.9,
                    "metrics": {"technical": 0.85},
                },
            ]
        }

        state.set_best_shot_results(rankings)

        assert state.best_shot_scores_by_path["sel1.jpg"]["cluster_id"] == 0
        assert state.best_shot_winners[0]["image_path"] == "sel1.jpg"

        # Removing a selection-only image should clear the cached rankings
        state.remove_data_for_path("sel1.jpg")
        assert state.best_shot_rankings[0][0]["image_path"] == "sel2.jpg"
