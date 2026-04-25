import requests
import time

print("Fetching active polymarket markets containing 'bitcoin'...")
url = "https://gamma-api.polymarket.com/markets?limit=200&active=true&closed=false"
r = requests.get(url)
data = r.json()

matches = []
for m in data:
    if "btc" in m["slug"].lower() or "bitcoin" in m["slug"].lower():
        matches.append(m["slug"])

for s in matches[:30]:
    print(s)
