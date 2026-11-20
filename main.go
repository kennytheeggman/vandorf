package main

import (
	"log"
	"net/http"
)

func main() {
	// 1. Define the directory containing your static files
	// "." points to the current working directory
	fileServer := http.FileServer(http.Dir("."))

	// 2. Register the file server handler for all routes
	http.Handle("/", fileServer)

	// 3. Start the server on port 8080
	log.Println("Starting static server on http://localhost:8080...")
	err := http.ListenAndServe(":80", nil)
	if err != nil {
		log.Fatal("Server failed to start: ", err)
	}
}

