-- Hand-written baseline for comparison with the search (not from the video).
-- A standard eco start: two mexes and energy, then the lab and one nano.
return {
  name = "eco first",
  source = "hand-written baseline",
  actions = { "mex", "mex", "wind", "wind", "wind", "lab", "wind", "wind", "nano" },
}
