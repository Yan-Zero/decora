# Input: one {provider, answers} object per line; no probabilities are recomputed.
# Display only: values are rounded to 3 decimals; the raw JSON lines above keep full precision.
def r3: (. * 1000 | round) / 1000 | tostring;
def display_width:
  [explode[] | if . >= 4352 then 2 else 1 end] | add // 0;
def cell($width):
  tostring | . + (" " * ([$width - display_width, 0] | max));
def row:
  (.[0] | cell(34)) + " | " +
  (.[1] | cell(9)) + " | " +
  (.[2] | cell(20)) + " | " + .[3];
(["provider", "damaged p", "team choice (conf)", "urgency score (3 d.p.)"] | row),
("-" * 85),
(.[] |
  .answers as $a |
  [ .provider,
    ($a[] | select(.name == "damaged") | .probability | r3),
    ($a[] | select(.name == "team") | "\(.choice) (\(.confidence | r3))"),
    ($a[] | select(.name == "urgency") | .score | r3)
  ] | row)
