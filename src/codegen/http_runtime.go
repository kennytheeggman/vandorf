// The fixed part of a generated http_vandorf.go, after http_runtime.rs:
// JSON values and their parser, encoders and decoders for primitives,
// lists and maps, and a net/http server. Generated code adds an encoder and
// decoder per type and a handler per server. Not compiled here: included
// as text by codegen::go.
//
// Requests: a route's proc takes its parameters from a JSON object body,
// or from the query string (each value read as JSON if it parses, else as
// a string). Responses: 200 and the result as JSON; 400 for a request that
// doesn't decode; 500 and {"error": ..} when the proc fails.

// httpNum is a JSON number as written.
type httpNum string

// httpObj is a JSON object, its fields in order. A JSON value is nil, a
// bool, an httpNum, a string, a []any or an httpObj.
type httpObj []httpPair

type httpPair struct {
	Key string
	Val any
}

func httpParse(s string) (any, error) {
	p := &httpParser{b: []byte(s)}
	v, err := p.value()
	if err != nil {
		return nil, err
	}
	p.ws()
	if p.i != len(p.b) {
		return nil, fmt.Errorf("unexpected input at byte %d", p.i)
	}
	return v, nil
}

func httpObject(v any, what string) (httpObj, error) {
	fs, ok := v.(httpObj)
	if !ok {
		return nil, fmt.Errorf("expected an object for %s", what)
	}
	return fs, nil
}

type httpParser struct {
	b []byte
	i int
}

func (p *httpParser) ws() {
	for p.i < len(p.b) && (p.b[p.i] == ' ' || p.b[p.i] == '\t' || p.b[p.i] == '\n' || p.b[p.i] == '\f' || p.b[p.i] == '\r') {
		p.i++
	}
}

func (p *httpParser) eat(c byte) error {
	p.ws()
	if p.i < len(p.b) && p.b[p.i] == c {
		p.i++
		return nil
	}
	return fmt.Errorf("expected '%c' at byte %d", c, p.i)
}

func (p *httpParser) value() (any, error) {
	p.ws()
	if p.i >= len(p.b) {
		return nil, errors.New("unexpected end of input")
	}
	switch c := p.b[p.i]; {
	case c == 'n':
		return p.word("null", nil)
	case c == 't':
		return p.word("true", true)
	case c == 'f':
		return p.word("false", false)
	case c == '"':
		return p.string()
	case c == '[':
		p.i++
		xs := []any{}
		p.ws()
		if p.i < len(p.b) && p.b[p.i] == ']' {
			p.i++
			return xs, nil
		}
		for {
			x, err := p.value()
			if err != nil {
				return nil, err
			}
			xs = append(xs, x)
			p.ws()
			if p.i < len(p.b) && p.b[p.i] == ',' {
				p.i++
			} else if p.i < len(p.b) && p.b[p.i] == ']' {
				p.i++
				return xs, nil
			} else {
				return nil, fmt.Errorf("expected ',' or ']' at byte %d", p.i)
			}
		}
	case c == '{':
		p.i++
		fs := httpObj{}
		p.ws()
		if p.i < len(p.b) && p.b[p.i] == '}' {
			p.i++
			return fs, nil
		}
		for {
			p.ws()
			k, err := p.string()
			if err != nil {
				return nil, err
			}
			if err := p.eat(':'); err != nil {
				return nil, err
			}
			v, err := p.value()
			if err != nil {
				return nil, err
			}
			fs = append(fs, httpPair{k, v})
			p.ws()
			if p.i < len(p.b) && p.b[p.i] == ',' {
				p.i++
			} else if p.i < len(p.b) && p.b[p.i] == '}' {
				p.i++
				return fs, nil
			} else {
				return nil, fmt.Errorf("expected ',' or '}' at byte %d", p.i)
			}
		}
	case c == '-' || (c >= '0' && c <= '9'):
		start := p.i
		p.i++
		for p.i < len(p.b) && strings.IndexByte("0123456789.eE+-", p.b[p.i]) >= 0 {
			p.i++
		}
		return httpNum(p.b[start:p.i]), nil
	}
	return nil, fmt.Errorf("unexpected character at byte %d", p.i)
}

func (p *httpParser) word(w string, v any) (any, error) {
	if !strings.HasPrefix(string(p.b[p.i:]), w) {
		return nil, fmt.Errorf("unexpected input at byte %d", p.i)
	}
	p.i += len(w)
	return v, nil
}

func (p *httpParser) string() (string, error) {
	if p.i >= len(p.b) || p.b[p.i] != '"' {
		return "", fmt.Errorf("expected a string at byte %d", p.i)
	}
	p.i++
	var out []byte
	for {
		if p.i >= len(p.b) {
			return "", errors.New("unterminated string")
		}
		c := p.b[p.i]
		switch c {
		case '"':
			p.i++
			return string(out), nil
		case '\\':
			if p.i+1 >= len(p.b) {
				return "", errors.New("unterminated string")
			}
			e := p.b[p.i+1]
			p.i += 2
			switch e {
			case 'n':
				out = append(out, '\n')
			case 't':
				out = append(out, '\t')
			case 'r':
				out = append(out, '\r')
			case 'b':
				out = append(out, 8)
			case 'f':
				out = append(out, 12)
			case 'u':
				if p.i+4 > len(p.b) {
					return "", errors.New("bad \\u escape")
				}
				cp, ok := httpHex(string(p.b[p.i : p.i+4]))
				if !ok {
					return "", errors.New("invalid digit found in string")
				}
				p.i += 4
				r := rune(cp)
				if !utf8.ValidRune(r) {
					r = utf8.RuneError
				}
				out = utf8.AppendRune(out, r)
			default:
				out = append(out, e)
			}
		default:
			out = append(out, c)
			p.i++
		}
	}
}

// httpHex reads hex digits, after an optional +, as Rust's from_str_radix.
func httpHex(s string) (uint64, bool) {
	s = strings.TrimPrefix(s, "+")
	if s == "" {
		return 0, false
	}
	var n uint64
	for i := 0; i < len(s); i++ {
		c := s[i]
		var d byte
		switch {
		case c >= '0' && c <= '9':
			d = c - '0'
		case c >= 'a' && c <= 'f':
			d = c - 'a' + 10
		case c >= 'A' && c <= 'F':
			d = c - 'A' + 10
		default:
			return 0, false
		}
		n = n*16 + uint64(d)
	}
	return n, true
}

// ---- Encoders and decoders ----

func httpWriteStr(b *strings.Builder, s string) {
	b.WriteByte('"')
	for _, c := range s {
		switch {
		case c == '"':
			b.WriteString("\\\"")
		case c == '\\':
			b.WriteString("\\\\")
		case c == '\n':
			b.WriteString("\\n")
		case c == '\r':
			b.WriteString("\\r")
		case c == '\t':
			b.WriteString("\\t")
		case c < 0x20:
			fmt.Fprintf(b, "\\u%04x", c)
		default:
			b.WriteRune(c)
		}
	}
	b.WriteByte('"')
}

func httpWriteVoid(b *strings.Builder, _ struct{}) { b.WriteString("null") }

func httpWriteInt(b *strings.Builder, x int) { b.WriteString(strconv.Itoa(x)) }

// httpWriteFloat writes x as Rust's {:?} does: the shortest digits that
// read back as x, in exponent form below 1e-4 or from 1e16 on.
func httpWriteFloat(b *strings.Builder, x float64) {
	if math.IsInf(x, 0) || math.IsNaN(x) {
		b.WriteString("null")
		return
	}
	if a := math.Abs(x); a != 0 && (a < 1e-4 || a >= 1e16) {
		s := strconv.FormatFloat(x, 'e', -1, 64)
		m, e, _ := strings.Cut(s, "e")
		n, _ := strconv.Atoi(e)
		fmt.Fprintf(b, "%se%d", m, n)
		return
	}
	s := strconv.FormatFloat(x, 'f', -1, 64)
	if !strings.Contains(s, ".") {
		s += ".0"
	}
	b.WriteString(s)
}

func httpWriteBool(b *strings.Builder, x bool) {
	if x {
		b.WriteString("true")
	} else {
		b.WriteString("false")
	}
}

func httpWriteString(b *strings.Builder, x string) { httpWriteStr(b, x) }

// httpListW encodes a list with f.
func httpListW[T any](f func(*strings.Builder, T)) func(*strings.Builder, []T) {
	return func(b *strings.Builder, xs []T) {
		b.WriteByte('[')
		for i, x := range xs {
			if i > 0 {
				b.WriteByte(',')
			}
			f(b, x)
		}
		b.WriteByte(']')
	}
}

// httpMapW encodes a map as an object, keys in order.
func httpMapW[K cmp.Ordered, V any](key func(K) string, f func(*strings.Builder, V)) func(*strings.Builder, map[K]V) {
	return func(b *strings.Builder, m map[K]V) {
		ks := make([]K, 0, len(m))
		for k := range m {
			ks = append(ks, k)
		}
		slices.Sort(ks)
		b.WriteByte('{')
		for i, k := range ks {
			if i > 0 {
				b.WriteByte(',')
			}
			httpWriteStr(b, key(k))
			b.WriteByte(':')
			f(b, m[k])
		}
		b.WriteByte('}')
	}
}

func httpIntKey(k int) string    { return strconv.Itoa(k) }
func httpStringKey(k string) string { return k }

func httpReadVoid(any) (struct{}, error) { return struct{}{}, nil }

func httpReadInt(v any) (int, error) {
	n, ok := v.(httpNum)
	if !ok {
		return 0, errors.New("expected an int")
	}
	x, err := strconv.ParseInt(string(n), 10, 64)
	if err != nil {
		return 0, fmt.Errorf("%s is not an int", n)
	}
	return int(x), nil
}

func httpReadFloat(v any) (float64, error) {
	n, ok := v.(httpNum)
	if !ok {
		return 0, errors.New("expected a float")
	}
	// Out of range reads as infinity or zero, as in Rust.
	x, err := strconv.ParseFloat(string(n), 64)
	if err != nil && !errors.Is(err, strconv.ErrRange) {
		return 0, fmt.Errorf("%s is not a float", n)
	}
	return x, nil
}

func httpReadBool(v any) (bool, error) {
	x, ok := v.(bool)
	if !ok {
		return false, errors.New("expected a bool")
	}
	return x, nil
}

func httpReadString(v any) (string, error) {
	x, ok := v.(string)
	if !ok {
		return "", errors.New("expected a string")
	}
	return x, nil
}

// httpListR decodes a list with f.
func httpListR[T any](f func(any) (T, error)) func(any) ([]T, error) {
	return func(v any) ([]T, error) {
		xs, ok := v.([]any)
		if !ok {
			return nil, errors.New("expected an array")
		}
		out := make([]T, 0, len(xs))
		for _, x := range xs {
			y, err := f(x)
			if err != nil {
				return nil, err
			}
			out = append(out, y)
		}
		return out, nil
	}
}

// httpMapR decodes an object as a map.
func httpMapR[K cmp.Ordered, V any](key func(string) (K, error), f func(any) (V, error)) func(any) (map[K]V, error) {
	return func(v any) (map[K]V, error) {
		fs, err := httpObject(v, "a map")
		if err != nil {
			return nil, err
		}
		m := make(map[K]V, len(fs))
		for _, kv := range fs {
			k, err := key(kv.Key)
			if err != nil {
				return nil, err
			}
			x, err := f(kv.Val)
			if err != nil {
				return nil, err
			}
			m[k] = x
		}
		return m, nil
	}
}

func httpIntFromKey(k string) (int, error) {
	x, err := strconv.ParseInt(k, 10, 64)
	if err != nil {
		return 0, fmt.Errorf("map key %s is not an int", k)
	}
	return int(x), nil
}

func httpStringFromKey(k string) (string, error) { return k, nil }

func httpField(fs httpObj, name string) (any, error) {
	for _, kv := range fs {
		if kv.Key == name {
			return kv.Val, nil
		}
	}
	return nil, fmt.Errorf("missing field %s", name)
}

// httpArg is a route's parameter, decoded; a failure is a 400.
func httpArg[T any](params any, name string, f func(any) (T, error)) (T, error) {
	var zero T
	fs, err := httpObject(params, "the parameters")
	if err != nil {
		return zero, err
	}
	for _, kv := range fs {
		if kv.Key == name {
			x, err := f(kv.Val)
			if err != nil {
				return zero, fmt.Errorf("parameter %s: %s", name, err)
			}
			return x, nil
		}
	}
	return zero, fmt.Errorf("missing parameter %s", name)
}

func httpJSON[T any](f func(*strings.Builder, T), x T) string {
	var b strings.Builder
	f(&b, x)
	return b.String()
}

// ---- The server ----

// HTTPRequest is a request: header names lowercased, the query string
// undecoded.
type HTTPRequest struct {
	Method  string
	Path    string
	Query   string
	Headers map[string]string
	Body    string
}

type HTTPResponse struct {
	Status  int
	Headers map[string]string
	Body    string
}

// HTTPHandler is a server's routes: the response, and whether it has the
// route.
type HTTPHandler func(ctx context.Context, req *HTTPRequest) (HTTPResponse, bool)

// HTTPStatic is a file served as it is.
type HTTPStatic struct {
	Path string
	Type string
	Body string
}

func httpJSONResponse(status int, body string) HTTPResponse {
	return HTTPResponse{Status: status, Headers: map[string]string{"Content-Type": "application/json"}, Body: body}
}

func httpErrorResponse(status int, msg string) HTTPResponse {
	var b strings.Builder
	b.WriteString("{\"error\":")
	httpWriteStr(&b, msg)
	b.WriteByte('}')
	return httpJSONResponse(status, b.String())
}

// httpQueryPairs is the query string's parameters, decoded.
func httpQueryPairs(query string) [][2]string {
	var out [][2]string
	for _, kv := range strings.Split(query, "&") {
		if kv == "" {
			continue
		}
		k, v, _ := strings.Cut(kv, "=")
		out = append(out, [2]string{httpPercentDecode(k), httpPercentDecode(v)})
	}
	return out
}

func httpQueryMap(query string) map[string]string {
	m := map[string]string{}
	for _, kv := range httpQueryPairs(query) {
		m[kv[0]] = kv[1]
	}
	return m
}

// httpJSONRoute is a JSON route: its parameters from a JSON object body,
// or the query string (each value read as JSON if it parses, else as a
// string).
func httpJSONRoute(ctx context.Context, req *HTTPRequest, call func(context.Context, any) (string, int, error)) HTTPResponse {
	var params any
	if strings.TrimSpace(req.Body) == "" {
		fs := httpObj{}
		for _, kv := range httpQueryPairs(req.Query) {
			v, err := httpParse(kv[1])
			if err != nil {
				v = kv[1]
			}
			fs = append(fs, httpPair{kv[0], v})
		}
		params = fs
	} else {
		v, err := httpParse(req.Body)
		if err != nil {
			return httpErrorResponse(400, "body: "+err.Error())
		}
		params = v
	}
	text, status, err := call(ctx, params)
	if err != nil {
		return httpErrorResponse(status, err.Error())
	}
	return httpJSONResponse(200, text)
}

// HTTPRun is a server's main: [--addr host:port] from the command line
// (else addr, the manifest's), then serves.
func HTTPRun(addr string, handler HTTPHandler, statics []HTTPStatic) {
	if a, ok := httpListenAddr(addr); ok {
		httpServe(a, handler, statics)
	}
}

// HTTPRunSetup is a server's main with a setup: the address as for
// HTTPRun, then the setup (if it fails, the server exits without
// listening), then serves.
func HTTPRunSetup(addr string, setup func(context.Context) error, handler HTTPHandler, statics []HTTPStatic) {
	a, ok := httpListenAddr(addr)
	if !ok {
		return
	}
	if err := setup(context.Background()); err != nil {
		fmt.Fprintf(os.Stderr, "error: setup: %v\n", err)
		os.Exit(1)
	}
	httpServe(a, handler, statics)
}

// httpListenAddr is --addr host:port from the command line, else addr;
// false after printing the usage for --help.
func httpListenAddr(addr string) (string, bool) {
	name := os.Args[0][strings.LastIndex(os.Args[0], "/")+1:]
	usage := fmt.Sprintf("usage: %s [--addr host:port]   (default %s)", name, addr)
	args := os.Args[1:]
	for i := 0; i < len(args); i += 2 {
		a := args[i]
		switch {
		case a == "--addr" && i+1 < len(args):
			addr = args[i+1]
		case a == "-h" || a == "--help":
			fmt.Fprintln(os.Stderr, usage)
			return "", false
		default:
			fmt.Fprintf(os.Stderr, "error: unexpected %s\n%s\n", a, usage)
			os.Exit(2)
		}
	}
	return addr, true
}

// httpServe serves until the process is stopped.
func httpServe(addr string, handler HTTPHandler, statics []HTTPStatic) {
	ln, err := net.Listen("tcp", addr)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: cannot listen on %s: %v\n", addr, err)
		os.Exit(1)
	}
	fmt.Fprintf(os.Stderr, "listening on http://%s\n", addr)
	http.Serve(ln, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(io.LimitReader(r.Body, 64<<20))
		path, query, _ := strings.Cut(r.RequestURI, "?")
		headers := map[string]string{}
		for k, vs := range r.Header {
			if len(vs) > 0 {
				headers[strings.ToLower(k)] = vs[len(vs)-1]
			}
		}
		if r.Host != "" {
			headers["host"] = r.Host
		}
		req := &HTTPRequest{Method: r.Method, Path: path, Query: query, Headers: headers, Body: strings.ToValidUTF8(string(body), "�")}
		resp := httpRespond(r.Context(), req, handler, statics)
		if resp.Status < 100 || resp.Status > 999 {
			resp = httpErrorResponse(500, fmt.Sprintf("status %d is not an HTTP status", resp.Status))
		}
		for k, v := range resp.Headers {
			w.Header()[k] = []string{v}
		}
		// Send no Content-Type the response doesn't set.
		if _, ok := w.Header()["Content-Type"]; !ok {
			w.Header()["Content-Type"] = nil
		}
		w.WriteHeader(resp.Status)
		io.WriteString(w, resp.Body)
	}))
}

func httpRespond(ctx context.Context, req *HTTPRequest, handler HTTPHandler, statics []HTTPStatic) HTTPResponse {
	if req.Method == "GET" {
		for _, s := range statics {
			if s.Path == req.Path {
				return HTTPResponse{Status: 200, Headers: map[string]string{"Content-Type": s.Type}, Body: s.Body}
			}
		}
	}
	if resp, ok := handler(ctx, req); ok {
		return resp
	}
	return httpErrorResponse(404, fmt.Sprintf("no route %s %s", req.Method, req.Path))
}

func httpPercentDecode(s string) string {
	var out []byte
	for i := 0; i < len(s); i++ {
		switch c := s[i]; {
		case c == '+':
			out = append(out, ' ')
		case c == '%' && i+2 < len(s):
			if x, ok := httpHex(s[i+1 : i+3]); ok {
				out = append(out, byte(x))
				i += 2
			} else {
				out = append(out, '%')
			}
		default:
			out = append(out, c)
		}
	}
	return strings.ToValidUTF8(string(out), "�")
}
