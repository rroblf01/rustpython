import hmac
print("hmac loaded")
h = hmac.HMAC(b"k", b"m", "sha256")
print("HMAC created")
print("hexdigest:", h.hexdigest())
print("OK")
