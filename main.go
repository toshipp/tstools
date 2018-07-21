package main

import (
	"fmt"
	"os"

	"./delay"
	"./split"
)

func main() {
	if len(os.Args) < 2 {
		fmt.Println("select delay or split subcommand")
		os.Exit(1)
	}
	switch os.Args[1] {
	case "delay":
		delay.Main(os.Args[2:])
	case "split":
		split.Main(os.Args[2:])
	}
}
