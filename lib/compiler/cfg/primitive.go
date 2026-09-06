package cfg

/** Identifier grammar rules **/

type Ident struct {
	Path []string `parser:"(@Ident '.')* @Ident" json:"path"`
}
