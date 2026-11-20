# Vendored oxideav-aac 0.1.7 (MIT)

Upstream: https://github.com/OxideAV/oxideav-aac (version 0.1.7). The tests directory was left
out; everything else is upstream's, with one change:

- `src/filterbank.rs`: `forward_mdct` and `imdct` read a cosine table computed once per
  transform length instead of calling `cos()` for every term. The values and the order of the
  additions are the same, so the output is unchanged, but encoding is far faster (the direct form
  took about 2.4 s per second of audio).
