# Web default fonts

Unmodified IBM Plex Sans and Lilex fonts previously embedded by GPUI CE at
`99109e3a77ddadbcb46cba8567e779a651600608`. Rake loads these before opening
a browser window because gpui-pre starts with an empty font database.
Native builds do not embed them. This preserves the previous default families
and regular, italic, bold/semibold and bold/semibold italic styles.

Both families use the SIL Open Font License. Copyright and license notices are
in [rake-web-font-licenses.txt](LICENSES.txt),
which the Web build tools include in their output.

SHA-256:

- `IBMPlexSans-Italic.ttf`: `a9c6ef9942c49e49d11e11a6dacc0b3a087978757e9b22a06b8ac22a6400fb15`
- `IBMPlexSans-Regular.ttf`: `975dcda37d80f038dcd143c22e33ca2d97a0cc5a929aace1c749153b0fe1afa5`
- `IBMPlexSans-SemiBold.ttf`: `a20caf8286023a6a7a85e40b1d2a4ae9fc3e3b1f9eda8f4c542dd4986af67bb1`
- `IBMPlexSans-SemiBoldItalic.ttf`: `147106b619423a82b40f6d64bace074ad5b786e59f48023bda69d63b1755fb82`
- `Lilex-Bold.ttf`: `3369214e2ecddb45ae5024b11d45dd637e99e9d2127ab729817d41d72b8a9c68`
- `Lilex-BoldItalic.ttf`: `9629bd65ee7376879414560e37281653ca82dcf33e143d7473341b2f88206985`
- `Lilex-Italic.ttf`: `a6e52046b91ed12a6af50bb74e70e27dea4a54eab2bed8947db4c1128ba83d45`
- `Lilex-Regular.ttf`: `8a467ad5a1941dc7c9cebaee34e3ed9364a73b48393034e363e4b528bd711d04`
