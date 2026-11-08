;; Script-driven in-memory disk creation, formatting, and hard-reset survival.
(import (scheme base) (neetan automation 1) (neetan test 1))

(test-suite "PC-98 in-memory disks"
  (with-machine (machine '((model . PC9801VM)))
    (test-case "media-create! mounts a blank RAM hard disk"
      (media-create! machine 'hdd 0 'sasi-40)
      (let ((info (media-info machine 'hdd 0)))
        (check-true (pair? info))
        (check-equal 'hdd (alist-ref info 'type))))

    (test-case "media-format! succeeds with a PC-98 partition table"
      (check-true (media-format! machine 'hdd 0 'pc98)))

    (test-case "media-create! mounts a blank RAM floppy as a D88 image"
      (media-create! machine 'floppy 0 '|2hd|)
      (let ((info (media-info machine 'floppy 0)))
        (check-true (pair? info))
        (check-equal "D88" (alist-ref info 'format))))

    (test-case "media-format! formats the floppy without a partition table"
      (check-true (media-format! machine 'floppy 0)))

    (test-case "the formatted disks survive a hard reset"
      (reset! machine 'hard)
      (check-true (pair? (media-info machine 'hdd 0)))
      (check-true (pair? (media-info machine 'floppy 0))))))
