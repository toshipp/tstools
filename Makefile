tssplit: tssplit.go libts/ts.go
	go build $<

tsdelay: tsdelay.go libts/ts.go
	go build $<

fmt:
	find . -name "*.go" -exec go fmt {} \;

.PHONY: fmt
