import time
import py_shot_limit as shot_limit
import pytest


@pytest.fixture
def bucket_fixture():
    # Create a bucket with a capacity of 2, and that refills 1 token every
    # 5 seconds
    return shot_limit.TokenBucket(capacity=2, increment=1, period_secs=5)


# First two calls should succeed (burst)
# Third call should fail
# Wait for 6 seconds to let the bucket refill...
# Fourth call should now succeed
# Fifth call should fail again
def test_with_bucket(bucket_fixture):
    assert bucket_fixture.process()
    assert bucket_fixture.process()
    assert bucket_fixture.process() is False
    time.sleep(6)
    assert bucket_fixture.process()
    assert bucket_fixture.process() is False
