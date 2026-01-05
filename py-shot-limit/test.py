import time
import py_shot_limit as shot_limit

print("--- Testing TokenBucket ---")
# Create a bucket with a capacity of 2, and that refills 1 token every 5 seconds
# Note: 1 token = 1,000,000,000 units
bucket = shot_limit.TokenBucket(capacity=2, increment=1, period_secs=5)

# First two calls should succeed (burst)
print("\nCall 1:", "Allowed" if bucket.process() else "Denied")

print("\nCall 2:", "Allowed" if bucket.process() else "Denied")

# Third call should fail
print("\nCall 3:", "Allowed" if bucket.process() else "Denied")


print("\nWaiting for 6 seconds to let the bucket refill...")
time.sleep(6)

# Fourth call should now succeed
print("\nCall 4:", "Allowed" if bucket.process() else "Denied")

# Fifth call should fail again
print("\nCall 5:", "Allowed" if bucket.process() else "Denied")

print("\n--- Testing FixedWindow ---")
# Create a fixed window limiter allowing 2 requests every 1 second
fw = shot_limit.FixedWindow(capacity=2, interval_secs=1)

print("\nFixedWindow Call 1:", "Allowed" if fw.process() else "Denied")
print("FixedWindow Call 2:", "Allowed" if fw.process() else "Denied")
print("FixedWindow Call 3:", "Allowed" if fw.process() else "Denied") # Should be denied

print("\nWaiting for 1.1 seconds for new window...")
time.sleep(1.1)

print("FixedWindow Call 4:", "Allowed" if fw.process() else "Denied") # Should be allowed
print("FixedWindow Call 5:", "Allowed" if fw.process() else "Denied") # Should be allowed
print("FixedWindow Call 6:", "Allowed" if fw.process() else "Denied") # Should be denied

print("\n--- Testing Gcra ---")
# Create a GCRA limiter allowing 2 requests every 1 second
gcra = shot_limit.Gcra(limit=2, period_secs=1)

print("\nGcra Call 1:", "Allowed" if gcra.process() else "Denied")
print("Gcra Call 2:", "Allowed" if gcra.process() else "Denied")
print("Gcra Call 3:", "Allowed" if gcra.process() else "Denied") # Should be denied

print("\nWaiting for 0.6 seconds...") # Should allow 1 request (half of 1 second period)
time.sleep(0.6)
print("Gcra Call 4:", "Allowed" if gcra.process() else "Denied") # Should be allowed
print("Gcra Call 5:", "Allowed" if gcra.process() else "Denied") # Should be denied


print("\n--- Testing SlidingWindow ---")
# Create a SlidingWindow limiter allowing 2 requests every 1 second
sw = shot_limit.SlidingWindow(capacity=2, interval_secs=1)

print("\nSlidingWindow Call 1:", "Allowed" if sw.process() else "Denied")
print("SlidingWindow Call 2:", "Allowed" if sw.process() else "Denied")
print("SlidingWindow Call 3:", "Allowed" if sw.process() else "Denied") # Should be denied

print("\nWaiting for 1.1 seconds for new window to establish and some weight to pass...")
time.sleep(1.1)

# At this point, the window should have slid.
# The previous window had 2 hits. The new window has 0.
# Estimated count should be less than capacity (e.g., 2 * (remaining_weight) + 0)
print("SlidingWindow Call 4:", "Allowed" if sw.process() else "Denied") # Should be allowed
print("SlidingWindow Call 5:", "Allowed" if sw.process() else "Denied") # Should be allowed
print("SlidingWindow Call 6:", "Allowed" if sw.process() else "Denied") # Should be denied
print("\nTest complete.")
