-- Hand-written baseline for comparison with the search (not from the video).
-- The commander builds the lab first, then energy and two mexes, then guards
-- the lab.
return {
  name = "lab first",
  source = "hand-written baseline",
  actions = { "lab", "wind", "wind", "mex", "mex", "wind", "wind", "wind", "wind" },
}
