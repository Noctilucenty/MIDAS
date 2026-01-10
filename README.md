# MIDAS

## Overview

MIDAS is an experimental **market data processing, strategy development, and backtesting framework** built primarily in **C++**, with supporting **Python utilities** for data handling and experimentation. The repository contains multiple Visual Studio–based projects that together form a research and prototyping environment for quantitative trading ideas.

The codebase focuses on:

* Efficient **data handling and preprocessing**
* **Strategy logic prototyping** in C++
* **Simulation / testing** of strategies against historical datasets
* Iterative experimentation rather than production deployment

> ⚠️ This repository is research-oriented and not intended to be production-ready trading software.

---

## Repository Structure

```
MIDAS/
├── Reconquista - skeleton/
│   └── Reconquista - Copy/
│       ├── Reconquista.sln
│       └── Reconquista/
│           ├── test.cpp
│           └── trade.cpp
│
├── data_handler/
│   └── data_handler/
│       ├── a.py
│       ├── read_me.txt
│       ├── data_handler.sln
│       └── data_handler/
│           └── main.cpp
│
├── st_tester/
│   └── st_tester/
│       ├── data_collector.py
│       ├── read_me.txt
│       ├── st_tester.sln
│       └── st_tester/
│           └── main.cpp
│
├── .gitignore
├── LICENSE
└── README.md
```

### Components

#### 1. Reconquista (Strategy Core)

* C++ Visual Studio project
* Contains core **strategy and trading logic**
* Used for testing different versions of algorithms and execution logic
* Focused on performance and low-level control

#### 2. data_handler (Data Processing)

* Hybrid **C++ + Python** module
* Responsible for:

  * Reading and transforming raw datasets
  * Preparing data for strategy testing
  * Rapid experimentation via Python utilities

#### 3. st_tester (Strategy Tester)

* C++ testing harness
* Designed to:

  * Run strategies against historical data
  * Evaluate outputs and performance metrics
  * Support iterative testing cycles

---

## Technologies Used

* **C++ (MSVC / Visual Studio)** – core logic, performance-critical code
* **Python** – data handling, preprocessing, and experimentation
* **Visual Studio** – project management and builds

---

## Build & Run

### Requirements

* Windows
* Visual Studio 2022 (or compatible version)
* Python 3.x (for data utilities)

### Build Steps

1. Open the desired `.sln` file in Visual Studio
2. Select `x64 | Debug` or `Release`
3. Build the solution
4. Run the executable from Visual Studio

---

## Datasets

Large datasets are **not included** in this repository.

If required:

* Download datasets separately
* Place them in the expected local directory (documented per module)

This keeps the repository lightweight and GitHub-friendly.

---

## Disclaimer

This project is for **educational and research purposes only**.
It is **not financial advice**, and it should not be used to trade real capital.

---

## License

See the `LICENSE` file for details.
