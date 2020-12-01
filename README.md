#### Starting the first peer with messaging period 5 seconds at port 8080:
./peer --period=5 --port=8080

#### Starting the second peer which will connect to the first
./peer --period=6 --port=8081 --connect="127.0.0.1:8080"


#### Starting the third peer which will connect to all the peers through the first
./peer --period=7 --port=8082 --connect="127.0.0.1:8080"
