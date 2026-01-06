import time
import py_shot_limit as shot_limit
import pytest


@pytest.fixture
def gcra_fixture():
    # Create a GCRA limiter allowing 2 requests every 1 second
    return shot_limit.Gcra(limit=2, period_secs=1)


# First two calls should succeed (burst)
# Third call should fail
# Wait for 0.6 seconds to let the bucket refill...
# Fourth call should now succeed
# Fifth call should fail again
def test_with_gcra(gcra_fixture):
    assert gcra_fixture.process()
    assert gcra_fixture.process()
    assert gcra_fixture.process() is False
    time.sleep(0.6)
    assert gcra_fixture.process()
    assert gcra_fixture.process() is False
