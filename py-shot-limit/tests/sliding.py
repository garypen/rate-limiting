import time
import py_shot_limit as shot_limit
import pytest


@pytest.fixture
def sliding_fixture():
    # Create a sliding window limiter allowing 2 requests every 1 second
    return shot_limit.FixedWindow(capacity=2, period_secs=1)


# First two calls should succeed (burst)
# Third call should fail
# Wait for 1.1 seconds to let the bucket refill...
# Fourth call should now succeed
# Fifth call should now succeed
# Sixth call should fail again
def test_with_sliding(sliding_fixture):
    assert sliding_fixture.process()
    assert sliding_fixture.process()
    assert sliding_fixture.process() is False
    time.sleep(1.1)
    assert sliding_fixture.process()
    assert sliding_fixture.process()
    assert sliding_fixture.process() is False
