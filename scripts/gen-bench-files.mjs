import { writeFileSync } from "fs";

function genCsv(rows) {
    const header = "id,name,email,age,score,active,signup_date\n";
    let body = "";
    for (let i = 0; i < rows; i++) {
        body += `${i},user${i},user${i}@example.com,${20 + (i % 50)},${(i % 100) / 10},${i % 2 === 0},2024-01-${String((i % 28) + 1).padStart(2, "0")}\n`;
    }
    return header + body;
}

writeFileSync("bench-1k.csv",   genCsv(1_000));
writeFileSync("bench-10k.csv",  genCsv(10_000));
writeFileSync("bench-500k.csv", genCsv(500_000));

console.log("Generated bench-1k.csv, bench-10k.csv, bench-500k.csv");
