(import
  (scheme base)
  (neetan automation 1)
  (neetan test 1))

(note "entering an infinite loop")
(let loop ()
  (loop))
