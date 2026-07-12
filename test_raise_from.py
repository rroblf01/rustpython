# Test raise from Y
try:
    raise ValueError("original") from RuntimeError("cause")
except ValueError as e:
    print("Caught ValueError")
    # Print cause info
    print("Done")
